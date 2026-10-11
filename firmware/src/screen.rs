use core::fmt::Write;
use core::sync::atomic::Ordering;

use embassy_futures::select::{Either, select};
use embassy_time::{Duration, Ticker, Timer};
use embedded_graphics::mono_font::iso_8859_1::{FONT_7X13_BOLD, FONT_9X18_BOLD};
use embedded_graphics::mono_font::{MonoFont, MonoTextStyle};
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::{Baseline, Text};
use esp_hal::Async;
use esp_hal::i2c::master::I2c;
use esp_hal::rng::Rng;
use heapless::String;
use ssd1306::prelude::*;
use ssd1306::{I2CDisplayInterface, Ssd1306Async};

use crate::net::{WifiState, wifi_state};
use crate::protocol::{Frame, Match, NORMAL_BRIGHTNESS, OwnStats, Player};
use crate::{DISPLAY_READY, FRAME};

const WIDTH: i32 = 128;
/// The panel's top 16 rows are yellow, the rest blue.
const HEADER_HEIGHT: i32 = 16;
/// Page markers in the header: two rows of four, one for each player a frame can carry.
const MAX_MARKERS: usize = 8;

/// Animation step; scrolling text moves one pixel per tick.
const TICK: Duration = Duration::from_millis(40);
/// How long scrolling text rests at either end.
const SCROLL_PAUSE: u32 = 25;
/// Shortest time a player stays on screen.
const MIN_PAGE_TICKS: u32 = 125;
/// How long the status screen stays up after power-on or a change of the WiFi state (30 s).
const STATUS_TICKS: u32 = 750;
/// The host repeats its frame every 5 s. After this long without one (30 s) it is gone.
const HOST_TIMEOUT_TICKS: u32 = 750;
/// Stands for the panel's lowest brightness, below the levels a match can ask for (1 to 3).
const IDLE_BRIGHTNESS: u8 = 0;
/// The idle tank drives one pixel every this many ticks, about 8 pixels a second.
const IDLE_STEP_TICKS: u32 = 3;

/// The idle tanks, facing right; they are mirrored to drive left. [`draw_idle`] cuts gaps
/// into the last row, so the tread appears to roll.
///
/// A German Tiger: low and boxy, with a long gun ending in a muzzle brake and a row of
/// large road wheels.
const TIGER: &[&str] = &[
    "............###.................",
    "..........############.......###",
    "..........######################",
    "..........############.......###",
    "..........############..........",
    "..##########################....",
    ".############################...",
    ".############################...",
    "###############################.",
    "#.###.###.###.###.###.###.###.#.",
    "#.###.###.###.###.###.###.###.#.",
    ".#############################..",
];
/// An American Sherman: tall, with a rounded turret, a short gun, a star on its side and
/// small wheels in three pairs.
const SHERMAN: &[&str] = &[
    "..........######............",
    ".........########...........",
    "........#################...",
    "........##########..........",
    ".....################.......",
    "...########.###########.....",
    "..########...############...",
    "..#########.##############..",
    ".##########################.",
    "##.##.##...##.##...##.##.###",
    "##.##.##...##.##...##.##.###",
    ".##########################.",
];

/// The idle tank: which one, where its top-left corner is, and its heading (+1 when it
/// drives right, -1 for left).
struct Tank {
    sprite: &'static [&'static str],
    at: Point,
    heading: i32,
}

impl Tank {
    fn width(&self) -> i32 {
        self.sprite[0].len() as i32
    }

    /// A tank of either side just outside the screen, about to drive across it on a random
    /// row of the blue area, from a random side.
    fn entering() -> Self {
        let random = Rng::new().random();
        let sprite = if random & (1 << 17) == 0 { TIGER } else { SHERMAN };
        let mut tank = Tank { sprite, at: Point::zero(), heading: if random & (1 << 16) == 0 { 1 } else { -1 } };
        let rows = (64 - HEADER_HEIGHT - sprite.len() as i32 + 1) as u32;
        let x = if tank.heading > 0 { -tank.width() } else { WIDTH };
        tank.at = Point::new(x, HEADER_HEIGHT + (random % rows) as i32);
        tank
    }

    /// Drives one pixel on. A tank that has left the screen makes way for the next one.
    fn advance(&mut self) {
        self.at.x += self.heading;
        if self.at.x < -self.width() || self.at.x > WIDTH {
            *self = Tank::entering();
        }
    }
}

/// What the screen is doing. An OLED wears where it is lit, so everything but a running
/// match ends in `Idle`.
enum Mode {
    /// WiFi state and address.
    Status { ticks_left: u32 },
    /// The match the host sent; `silent_ticks` counts the time since its last frame.
    Match { silent_ticks: u32 },
    /// The player's own ratings, which the host sends while the game sits in its menu.
    Stats { silent_ticks: u32 },
    /// A dimmed tank patrolling the blue area on changing rows, so no pixel is lit for long.
    Idle { ticks: u32, tank: Tank },
}

fn idle() -> Mode {
    Mode::Idle { ticks: 0, tank: Tank::entering() }
}

fn text<D: DrawTarget<Color = BinaryColor>>(target: &mut D, content: &str, at: Point, font: &MonoFont<'_>) {
    let style = MonoTextStyle::new(font, BinaryColor::On);
    let _ = Text::with_baseline(content, at, style, Baseline::Top).draw(target);
}

fn text_width(content: &str, font: &MonoFont<'_>) -> i32 {
    content.chars().count() as i32 * font.character_size.width as i32
}

/// Offset of text that is `overflow` pixels too wide: rest, scroll to the end, rest, scroll back.
fn scroll_offset(overflow: i32, tick: u32) -> i32 {
    if overflow <= 0 {
        return 0;
    }
    let overflow = overflow as u32;
    let phase = tick % (2 * SCROLL_PAUSE + 2 * overflow);
    let offset = if phase < SCROLL_PAUSE {
        0
    } else if phase < SCROLL_PAUSE + overflow {
        phase - SCROLL_PAUSE
    } else if phase < 2 * SCROLL_PAUSE + overflow {
        overflow
    } else {
        2 * SCROLL_PAUSE + 2 * overflow - phase
    };
    offset as i32
}

/// Draws text inside `area`, scrolling it when it does not fit. Returns the overflow in pixels.
fn scrolling_text<D: DrawTarget<Color = BinaryColor>>(
    target: &mut D,
    content: &str,
    area: Rectangle,
    font: &MonoFont<'_>,
    tick: u32,
) -> i32 {
    let overflow = (text_width(content, font) - area.size.width as i32).max(0);
    let origin = area.top_left - Point::new(scroll_offset(overflow, tick), 0);
    text(&mut target.clipped(&area), content, origin, font);
    overflow
}

/// A full-width line starting at `y`.
fn line_area(y: i32, font: &MonoFont<'_>) -> Rectangle {
    Rectangle::new(Point::new(0, y), Size::new(WIDTH as u32, font.character_size.height))
}

fn faction_name(code: char) -> &'static str {
    match code {
        'A' => "USF",
        'B' => "UKF",
        'D' => "DAK",
        'G' => "Wehr",
        _ => "?",
    }
}

/// `Crossing in the Woods`, plus one square per page with the current one filled; from
/// three pages on the squares are stacked in two rows.
/// Returns how far the map name overflows.
fn draw_header<D: DrawTarget<Color = BinaryColor>>(
    target: &mut D,
    current: &Match,
    page: usize,
    pages: usize,
    tick: u32,
) -> i32 {
    // The app sends the map's title. Before 0.3.3 of this firmware it sent the scenario's
    // file name (`crossing_in_the_woods`), which is all lower case and still gets tidied up.
    let is_title = current.map.chars().any(char::is_uppercase);
    let mut map: String<64> = String::new();
    let mut word_start = !is_title;
    for c in current.map.chars() {
        let c = if c == '_' && !is_title { ' ' } else { c };
        let _ = map.push(if word_start { c.to_ascii_uppercase() } else { c });
        word_start = c == ' ' && !is_title;
    }
    draw_title(target, &map, page, pages, tick)
}

/// The yellow line: a title, scrolling if it has to, and at its right end one square per
/// page. Returns how far the title overflows.
fn draw_title<D: DrawTarget<Color = BinaryColor>>(
    target: &mut D,
    title: &str,
    page: usize,
    pages: usize,
    tick: u32,
) -> i32 {
    let font = &FONT_7X13_BOLD;
    let markers = if pages > 1 { pages.min(MAX_MARKERS) as i32 } else { 0 };
    // Two pages sit side by side; more are stacked in two rows to leave the title its room.
    let rows = if markers > 2 { 2 } else { 1 };
    let columns = (markers + rows - 1) / rows;
    let markers_width = if markers > 0 { columns * 7 + 2 } else { 0 };

    let area = Rectangle::new(Point::new(1, 1), Size::new((WIDTH - markers_width - 1).max(0) as u32, 13));
    let overflow = scrolling_text(target, title, area, font, tick);

    for index in 0..markers {
        // Row by row, like text: 1 2 above 3 4.
        let x = WIDTH - (columns - index % columns) * 7;
        let y = if rows == 1 { 5 } else { 2 + index / columns * 7 };
        let square = Rectangle::new(Point::new(x, y), Size::new(5, 5));
        let style = if index as usize == page {
            PrimitiveStyle::with_fill(BinaryColor::On)
        } else {
            PrimitiveStyle::with_stroke(BinaryColor::On, 1)
        };
        let _ = square.into_styled(style).draw(target);
    }
    overflow
}

/// One player on the blue area:
///
/// ```text
/// Von_Dolofon
/// USF 1284 +3 GR
/// 164W 135L 55%
/// ```
///
/// Returns the widest overflow among the scrolling lines.
fn draw_player<D: DrawTarget<Color = BinaryColor>>(target: &mut D, player: &Player, tick: u32) -> i32 {
    // 13 + 18 + 18 rows is one more than the blue area has, so the rating line starts on the
    // name's last row; the big font's top rows are empty, so nothing collides.
    let name_font = &FONT_7X13_BOLD;
    let rating_font = &FONT_9X18_BOLD;
    let record_font = &FONT_9X18_BOLD;
    let (name_y, rating_y, record_y) = (HEADER_HEIGHT, HEADER_HEIGHT + 12, HEADER_HEIGHT + 30);

    let mut overflow = scrolling_text(target, &player.name, line_area(name_y, name_font), name_font, tick);

    let mut rating: String<24> = String::new();
    match player.elo {
        Some(elo) => {
            let _ = write!(rating, "{} {elo}", faction_name(player.faction));
            if let Some(streak) = player.streak {
                let _ = write!(rating, " {streak:+}");
            }
        }
        None => {
            let _ = write!(rating, "{} unrated", faction_name(player.faction));
        }
    }
    text(target, &rating, Point::new(0, rating_y), rating_font);

    let mut country: String<4> = String::new();
    for c in player.country.chars() {
        let _ = country.push(c.to_ascii_uppercase());
    }
    let country_x = WIDTH - text_width(&country, rating_font);
    if country_x > text_width(&rating, rating_font) {
        text(target, &country, Point::new(country_x, rating_y), rating_font);
    }

    let mut record: String<40> = String::new();
    if let (Some(wins), Some(losses)) = (player.wins, player.losses) {
        let _ = write!(record, "{wins}W {losses}L");
        if let Some(winrate) = player.winrate() {
            let _ = write!(record, " {winrate}%");
        }
    }
    overflow = overflow.max(scrolling_text(target, &record, line_area(record_y, record_font), record_font, tick));
    overflow
}

/// What [`draw`] found out about the page it drew.
#[derive(Default)]
struct Drawn {
    /// Widest overflow among the player's lines, in pixels.
    player_overflow: i32,
    /// Whether anything on the page scrolls and needs redrawing every tick.
    animated: bool,
}

/// Draws the status screen: what the display is waiting for and how its WiFi is doing.
fn draw_status<D: DrawTarget<Color = BinaryColor>>(target: &mut D) {
    let _ = target.clear(BinaryColor::Off);
    text(target, "CoH3", Point::new(1, 1), &FONT_7X13_BOLD);
    text(target, "Waiting for match", Point::new(0, HEADER_HEIGHT + 4), &FONT_7X13_BOLD);
    let mut wifi: String<24> = String::new();
    let _ = match wifi_state() {
        WifiState::Unset => write!(wifi, "WiFi not set up"),
        WifiState::Off => write!(wifi, "WiFi off"),
        WifiState::Connecting => write!(wifi, "WiFi connecting"),
        WifiState::Up([a, b, c, d]) => write!(wifi, "{a}.{b}.{c}.{d}"),
    };
    text(target, &wifi, Point::new(0, HEADER_HEIGHT + 28), &FONT_7X13_BOLD);
}

fn draw_idle<D: DrawTarget<Color = BinaryColor>>(target: &mut D, tank: &Tank) {
    let _ = target.clear(BinaryColor::Off);
    let pixels = tank.sprite.iter().enumerate().flat_map(|(row, line)| {
        line.bytes().enumerate().filter_map(move |(column, cell)| {
            let column = if tank.heading > 0 { column as i32 } else { tank.width() - 1 - column as i32 };
            let at = tank.at + Point::new(column, row as i32);
            // The tread's gaps stay where they are on the ground while the tank moves over them.
            let lit = cell == b'#' && (row + 1 != tank.sprite.len() || at.x.rem_euclid(3) != 0);
            lit.then_some(Pixel(at, BinaryColor::On))
        })
    });
    // The tank drives in and out of view; what is off the screen is not drawn.
    let blue = Rectangle::new(Point::new(0, HEADER_HEIGHT), Size::new(WIDTH as u32, (64 - HEADER_HEIGHT) as u32));
    let _ = target.clipped(&blue).draw_iter(pixels);
}

/// Draws one page of the player's own ratings: their name in the header, then a mode and
/// the rating for each faction:
///
/// ```text
/// 1v1
/// USF 1207   UKF 1150
///
/// DAK -     Wehr 1369
/// ```
fn draw_stats<D: DrawTarget<Color = BinaryColor>>(target: &mut D, own: &OwnStats, page: usize, tick: u32) -> Drawn {
    let _ = target.clear(BinaryColor::Off);
    let page = page % own.ratings.len();
    let overflow = draw_title(target, &own.name, page, own.ratings.len(), tick);

    text(target, if page == 0 { "1v1" } else { "2v2" }, Point::new(0, HEADER_HEIGHT), &FONT_9X18_BOLD);
    for (index, (rating, faction)) in own.ratings[page].iter().zip(['A', 'B', 'D', 'G']).enumerate() {
        let mut cell: String<12> = String::new();
        let _ = match rating {
            Some(rating) => write!(cell, "{} {rating}", faction_name(faction)),
            None => write!(cell, "{} -", faction_name(faction)),
        };
        // Two columns, the left one against the left edge and the right one against the
        // right edge, which keeps them apart: only "Wehr 1302" is nine characters long.
        let font = &FONT_7X13_BOLD;
        let x = if index % 2 == 0 { 0 } else { WIDTH - text_width(&cell, font) };
        // Two rows with 5 pixels between them. The upper one starts on the mode line's last
        // row, which is empty: its letters have no tails.
        text(target, &cell, Point::new(x, HEADER_HEIGHT + 17 + (index / 2) as i32 * 18), font);
    }
    Drawn { player_overflow: 0, animated: overflow > 0 }
}

/// Draws one page of a match: the header and one player.
fn draw_match<D: DrawTarget<Color = BinaryColor>>(
    target: &mut D,
    current: &Match,
    page: usize,
    header_tick: u32,
    page_tick: u32,
) -> Drawn {
    let _ = target.clear(BinaryColor::Off);

    let pages = current.players.len();
    if pages == 0 {
        let overflow = draw_header(target, current, 0, 1, header_tick);
        return Drawn { player_overflow: 0, animated: overflow > 0 };
    }
    let page = page % pages;
    let header_overflow = draw_header(target, current, page, pages, header_tick);
    let player_overflow = draw_player(target, &current.players[page], page_tick);

    Drawn { player_overflow, animated: header_overflow > 0 || player_overflow > 0 }
}

/// Owns the OLED: shows the match the host sends, flips between the players, scrolls long
/// text, and falls back to the dimmed idle tank when there is nothing current to show.
#[embassy_executor::task]
pub async fn screen_task(i2c: I2c<'static, Async>) {
    let interface = I2CDisplayInterface::new(i2c);
    let mut display =
        Ssd1306Async::new(interface, DisplaySize128x64, DisplayRotation::Rotate0).into_buffered_graphics_mode();
    while display.init().await.is_err() {
        Timer::after(Duration::from_secs(1)).await;
    }
    DISPLAY_READY.store(true, Ordering::Relaxed);

    let mut current: Option<Match> = None;
    let mut own: Option<OwnStats> = None;
    let mut page = 0usize;
    // The header scrolls independently of page flips; a player's lines restart with the page.
    let mut header_tick = 0u32;
    let mut page_tick = 0u32;
    let mut drawn = Drawn::default();
    let mut dirty = true;
    let mut mode = Mode::Status { ticks_left: STATUS_TICKS };
    let mut shown_wifi = wifi_state();
    // What the panel is set to: a match's level, or `IDLE_BRIGHTNESS`.
    let mut level = NORMAL_BRIGHTNESS;
    let mut ticker = Ticker::every(TICK);
    loop {
        if dirty {
            // Idle runs at the panel's lowest brightness, a match at the one the app asks for.
            let wanted = match (&mode, &current) {
                (Mode::Idle { .. }, _) => IDLE_BRIGHTNESS,
                (Mode::Match { .. }, Some(current)) => current.brightness,
                // The menu can stay up for hours, so the ratings are shown dimmed.
                (Mode::Stats { .. }, _) => 1,
                _ => NORMAL_BRIGHTNESS,
            };
            if wanted != level {
                let brightness = match wanted {
                    IDLE_BRIGHTNESS => Brightness::DIMMEST,
                    1 => Brightness::DIM,
                    3 => Brightness::BRIGHTEST,
                    _ => Brightness::NORMAL,
                };
                level = display.set_brightness(brightness).await.map_or(level, |()| wanted);
            }
            match (&mode, &current, &own) {
                (Mode::Match { .. }, Some(current), _) => {
                    drawn = draw_match(&mut display, current, page, header_tick, page_tick);
                }
                (Mode::Stats { .. }, _, Some(own)) => drawn = draw_stats(&mut display, own, page, header_tick),
                (Mode::Idle { tank, .. }, ..) => draw_idle(&mut display, tank),
                _ => draw_status(&mut display),
            }
            let _ = display.flush().await;
        }

        match select(FRAME.wait(), ticker.next()).await {
            Either::First(Frame::Match(received)) => {
                // The host repeats the current match; only a different one restarts the page.
                // A change of brightness alone is shown at once, on the page that is up.
                let changed = current.as_ref() != Some(&received);
                let same_match = current.as_ref().is_some_and(|shown| {
                    shown.mode == received.mode && shown.map == received.map && shown.players == received.players
                });
                dirty = changed || !matches!(mode, Mode::Match { .. });
                if !same_match {
                    page = 0;
                    header_tick = 0;
                    page_tick = 0;
                }
                if changed {
                    current = Some(received);
                }
                mode = Mode::Match { silent_ticks: 0 };
            }
            Either::First(Frame::Stats(received)) => {
                // The status screen runs out by itself; it is there to be read.
                if matches!(mode, Mode::Status { .. }) {
                    dirty = false;
                    continue;
                }
                let entering = !matches!(mode, Mode::Stats { .. });
                dirty = entering || own.as_ref() != Some(&received);
                if entering {
                    current = None;
                    page = 0;
                    header_tick = 0;
                    page_tick = 0;
                }
                own = Some(received);
                mode = Mode::Stats { silent_ticks: 0 };
            }
            Either::First(Frame::Idle) => {
                // Not the status screen either: it, too, runs out by itself.
                dirty = matches!(mode, Mode::Match { .. } | Mode::Stats { .. });
                if dirty {
                    current = None;
                    own = None;
                    mode = idle();
                }
            }
            Either::Second(()) => {
                let wifi = wifi_state();
                let wifi_changed = wifi != shown_wifi;
                shown_wifi = wifi;

                match &mut mode {
                    Mode::Match { silent_ticks } => {
                        *silent_ticks += 1;
                        if *silent_ticks > HOST_TIMEOUT_TICKS {
                            current = None;
                            mode = idle();
                            dirty = true;
                            continue;
                        }
                        header_tick = header_tick.wrapping_add(1);
                        page_tick += 1;
                        // Stay long enough for the longest line to scroll to its end and rest there.
                        let page_ticks = MIN_PAGE_TICKS.max(2 * SCROLL_PAUSE + drawn.player_overflow as u32);
                        let flip = page_tick >= page_ticks;
                        if flip {
                            page = page.wrapping_add(1);
                            page_tick = 0;
                        }
                        dirty = flip || drawn.animated;
                    }
                    // A change of the WiFi state is worth showing, for a while.
                    _ if wifi_changed => {
                        mode = Mode::Status { ticks_left: STATUS_TICKS };
                        dirty = true;
                    }
                    Mode::Status { ticks_left } => {
                        *ticks_left = ticks_left.saturating_sub(1);
                        dirty = *ticks_left == 0;
                        if dirty {
                            mode = idle();
                        }
                    }
                    Mode::Stats { silent_ticks } => {
                        *silent_ticks += 1;
                        if *silent_ticks > HOST_TIMEOUT_TICKS {
                            own = None;
                            mode = idle();
                            dirty = true;
                            continue;
                        }
                        header_tick = header_tick.wrapping_add(1);
                        page_tick += 1;
                        let flip = page_tick >= MIN_PAGE_TICKS;
                        if flip {
                            page = page.wrapping_add(1);
                            page_tick = 0;
                        }
                        dirty = flip || drawn.animated;
                    }
                    Mode::Idle { ticks, tank } => {
                        *ticks += 1;
                        dirty = *ticks >= IDLE_STEP_TICKS;
                        if dirty {
                            *ticks = 0;
                            tank.advance();
                        }
                    }
                }
            }
        }
    }
}
