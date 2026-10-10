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
use crate::protocol::{Frame, Match, Player};
use crate::{DISPLAY_READY, FRAME};

const WIDTH: i32 = 128;
/// The panel's top 16 rows are yellow, the rest blue.
const HEADER_HEIGHT: i32 = 16;
/// Page markers in the header; more players than this simply get no marker of their own.
const MAX_MARKERS: usize = 4;

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
/// The idle label glides one pixel every this many ticks, about 8 pixels a second.
const IDLE_STEP_TICKS: u32 = 3;
const IDLE_LABEL: &str = "CoH 3";
const IDLE_FONT: &MonoFont<'static> = &FONT_7X13_BOLD;

/// What the screen is doing. An OLED wears where it is lit, so everything but a running
/// match ends in `Idle`.
enum Mode {
    /// WiFi state and address.
    Status { ticks_left: u32 },
    /// The match the host sent; `silent_ticks` counts the time since its last frame.
    Match { silent_ticks: u32 },
    /// A dimmed label drifting slowly around the blue area, so no pixel is lit for long.
    Idle { ticks: u32, at: Point, step: Point },
}

/// Where the idle label's top-left corner may be: anywhere it fits below the yellow rows.
fn idle_bounds() -> Rectangle {
    let label = Size::new(text_width(IDLE_LABEL, IDLE_FONT) as u32, IDLE_FONT.character_size.height);
    Rectangle::new(
        Point::new(0, HEADER_HEIGHT),
        Size::new(WIDTH as u32 - label.width + 1, (64 - HEADER_HEIGHT) as u32 - label.height + 1),
    )
}

/// Starts idling at a random spot, heading in a random diagonal direction.
fn idle() -> Mode {
    let random = Rng::new().random();
    let bounds = idle_bounds();
    let at = bounds.top_left
        + Point::new((random % bounds.size.width) as i32, ((random >> 8) % bounds.size.height) as i32);
    let step = Point::new(if random & (1 << 16) == 0 { 1 } else { -1 }, if random & (1 << 17) == 0 { 1 } else { -1 });
    Mode::Idle { ticks: 0, at, step }
}

/// Moves the idle label one pixel on, turning round at the edges of its area.
fn drift(at: &mut Point, step: &mut Point) {
    let bounds = idle_bounds();
    let last = bounds.top_left + Point::new(bounds.size.width as i32 - 1, bounds.size.height as i32 - 1);
    if at.x + step.x < bounds.top_left.x || at.x + step.x > last.x {
        step.x = -step.x;
    }
    if at.y + step.y < bounds.top_left.y || at.y + step.y > last.y {
        step.y = -step.y;
    }
    *at += *step;
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

/// `Crossing In The Woods`, plus one square per page with the current one filled.
/// Returns how far the map name overflows.
fn draw_header<D: DrawTarget<Color = BinaryColor>>(
    target: &mut D,
    current: &Match,
    page: usize,
    pages: usize,
    tick: u32,
) -> i32 {
    let font = &FONT_7X13_BOLD;
    let markers = if pages > 1 { pages.min(MAX_MARKERS) as i32 } else { 0 };
    let markers_width = if markers > 0 { markers * 7 + 2 } else { 0 };

    let mut map: String<64> = String::new();
    let mut word_start = true;
    for c in current.map.chars() {
        let c = if c == '_' { ' ' } else { c };
        let _ = map.push(if word_start { c.to_ascii_uppercase() } else { c });
        word_start = c == ' ';
    }
    let area = Rectangle::new(Point::new(1, 1), Size::new((WIDTH - markers_width - 1).max(0) as u32, 13));
    let overflow = scrolling_text(target, &map, area, font, tick);

    for index in 0..markers {
        let square = Rectangle::new(Point::new(WIDTH - (markers - index) * 7, 5), Size::new(5, 5));
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

fn draw_idle<D: DrawTarget<Color = BinaryColor>>(target: &mut D, at: Point) {
    let _ = target.clear(BinaryColor::Off);
    text(target, IDLE_LABEL, at, IDLE_FONT);
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
/// text, and falls back to the dimmed idle label when there is nothing current to show.
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
    let mut page = 0usize;
    // The header scrolls independently of page flips; a player's lines restart with the page.
    let mut header_tick = 0u32;
    let mut page_tick = 0u32;
    let mut drawn = Drawn::default();
    let mut dirty = true;
    let mut mode = Mode::Status { ticks_left: STATUS_TICKS };
    let mut shown_wifi = wifi_state();
    let mut dimmed = false;
    let mut ticker = Ticker::every(TICK);
    loop {
        if dirty {
            // Idle runs at the panel's lowest brightness; everything else at the normal one.
            let idling = matches!(mode, Mode::Idle { .. });
            if idling != dimmed {
                let brightness = if idling { Brightness::DIMMEST } else { Brightness::NORMAL };
                dimmed = display.set_brightness(brightness).await.map_or(dimmed, |()| idling);
            }
            match (&mode, &current) {
                (Mode::Match { .. }, Some(current)) => {
                    drawn = draw_match(&mut display, current, page, header_tick, page_tick);
                }
                (Mode::Idle { at, .. }, _) => draw_idle(&mut display, *at),
                _ => draw_status(&mut display),
            }
            let _ = display.flush().await;
        }

        match select(FRAME.wait(), ticker.next()).await {
            Either::First(Frame::Match(received)) => {
                // The host repeats the current match; only a different one restarts the page.
                let is_new = current.as_ref() != Some(&received);
                dirty = is_new || !matches!(mode, Mode::Match { .. });
                if is_new {
                    current = Some(received);
                    page = 0;
                    header_tick = 0;
                    page_tick = 0;
                }
                mode = Mode::Match { silent_ticks: 0 };
            }
            Either::First(Frame::Idle) => {
                // The status screen runs out by itself; it is there to be read.
                dirty = matches!(mode, Mode::Match { .. });
                if dirty {
                    current = None;
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
                    Mode::Idle { ticks, at, step } => {
                        *ticks += 1;
                        dirty = *ticks >= IDLE_STEP_TICKS;
                        if dirty {
                            *ticks = 0;
                            drift(at, step);
                        }
                    }
                }
            }
        }
    }
}
