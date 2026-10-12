//! Finds displays on the LAN by their beacon and sends them frames over UDP.

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::thread;
use std::time::{Duration, Instant};

use display_protocol::Reply;
use display_protocol::udp::{
    BEACON_PORT, DISPLAY_TIMEOUT, Datagram, FRAME_PORT, PROBE, PROBE_INTERVAL, parse_beacon,
};

use super::{Ack, DISABLED_POLL, Every, LinkContext, RESEND, Round, Stop, WifiLink, match_frame};
use crate::applog::log;
use crate::i18n::Lang;
use crate::model::MatchInfo;
use crate::settings::Brightness;

const POLL: Duration = Duration::from_millis(200);

struct Device {
    last_seen: Instant,
    /// Given on the command line, so it is kept even without beacons.
    fixed: bool,
    /// A frame was sent and not answered yet.
    awaiting_ack: bool,
    reported: Option<Ack>,
}

impl Device {
    fn new(fixed: bool) -> Self {
        Self { last_seen: Instant::now(), fixed, awaiting_ack: false, reported: None }
    }

    fn report(&mut self, addr: IpAddr, ack: Ack) {
        if self.reported != Some(ack) {
            log(format!("Display at {addr}: {}", ack.describe()));
            self.reported = Some(ack);
        }
    }
}

/// Listens on the beacon port. With a fixed address the port is not essential, so any free
/// port will do when it is taken.
fn bind(fixed: Option<IpAddr>) -> io::Result<UdpSocket> {
    let socket = match UdpSocket::bind((Ipv4Addr::UNSPECIFIED, BEACON_PORT)) {
        Ok(socket) => socket,
        Err(_) if fixed.is_some() => UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?,
        Err(err) => return Err(err),
    };
    socket.set_read_timeout(Some(POLL))?;
    socket.set_broadcast(true)?;
    Ok(socket)
}

fn probe(socket: &UdpSocket) {
    let _ = socket.send_to(PROBE, (Ipv4Addr::BROADCAST, FRAME_PORT));
}

/// Handles everything that arrived on the socket within one poll interval. Returns whether a
/// display appeared that has not been sent the current frame yet.
fn receive(socket: &UdpSocket, devices: &mut HashMap<IpAddr, Device>) -> io::Result<bool> {
    let mut found_new = false;
    let mut buf = [0u8; 256];
    let deadline = Instant::now() + POLL;
    while Instant::now() < deadline {
        let (len, sender) = match socket.recv_from(&mut buf) {
            Ok(received) => received,
            Err(err) if matches!(err.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) => break,
            // Windows reports an ICMP "port unreachable" for an earlier send this way.
            Err(err) if err.kind() == io::ErrorKind::ConnectionReset => continue,
            Err(err) => return Err(err),
        };
        let data = &buf[..len];
        if let Some(id) = parse_beacon(data) {
            let device = devices.entry(sender.ip()).or_insert_with(|| {
                log(format!("Display found at {} ({id})", sender.ip()));
                found_new = true;
                Device::new(false)
            });
            device.last_seen = Instant::now();
        } else if let (Some(ack), Some(device)) = (Reply::parse(data), devices.get_mut(&sender.ip())) {
            device.awaiting_ack = false;
            device.report(sender.ip(), ack.into());
        }
    }
    Ok(found_new)
}

fn send(socket: &UdpSocket, devices: &mut HashMap<IpAddr, Device>, frame: &str) {
    for (addr, device) in devices.iter_mut() {
        if device.awaiting_ack {
            device.report(*addr, Ack::Silent);
        }
        device.awaiting_ack = socket.send_to(frame.as_bytes(), SocketAddr::new(*addr, FRAME_PORT)).is_ok();
    }
}

fn publish(ctx: &LinkContext, devices: &HashMap<IpAddr, Device>) {
    let mut links: Vec<WifiLink> =
        devices.iter().map(|(addr, device)| WifiLink { addr: *addr, ack: device.reported }).collect();
    links.sort_by_key(|link| link.addr);
    ctx.status.send_if_modified(|status| {
        let changed = status.wifi != links;
        if changed {
            status.wifi = links;
        }
        changed
    });
}

fn serve(socket: &UdpSocket, fixed: Option<IpAddr>, ctx: &mut LinkContext) -> io::Result<Stop> {
    let mut devices: HashMap<IpAddr, Device> = HashMap::new();
    if let Some(addr) = fixed {
        devices.insert(addr, Device::new(true));
    }
    let mut resend = Every::new(RESEND);
    let mut ask = Every::new(PROBE_INTERVAL);
    loop {
        let Round { settings, changed } = match ctx.round(|settings| settings.wifi) {
            Ok(round) => round,
            Err(stop) => return Ok(stop),
        };
        if ask.is_due() {
            probe(socket);
            ask.done();
        }
        let found_new = receive(socket, &mut devices)?;
        devices.retain(|addr, device| {
            let keep = device.fixed || device.last_seen.elapsed() < DISPLAY_TIMEOUT;
            if !keep {
                log(format!("Display at {addr} is gone"));
            }
            keep
        });

        if changed || found_new || resend.is_due() {
            let frame = ctx.frame(&settings);
            send(socket, &mut devices, &Datagram(&frame).to_string());
            resend.done();
        }
        publish(ctx, &devices);
    }
}

/// Starts the background thread that discovers displays and keeps them in sync with the
/// current match. `fixed` adds a display by address, for networks where discovery fails.
pub fn spawn(fixed: Option<IpAddr>, mut ctx: LinkContext) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut last_problem = String::new();
        loop {
            if ctx.matches.has_changed().is_err() {
                return;
            }
            if !ctx.settings.borrow().wifi {
                thread::sleep(DISABLED_POLL);
                continue;
            }
            let result = bind(fixed).and_then(|socket| serve(&socket, fixed, &mut ctx));
            publish(&ctx, &HashMap::new());
            match result {
                Ok(Stop::Closed) => return,
                // Firmware is only ever installed over USB.
                Ok(Stop::Disabled | Stop::Install) => log("WiFi displays switched off"),
                Err(err) => {
                    let problem = format!("WiFi display link failed: {err}");
                    if problem != last_problem {
                        log(&problem);
                        last_problem = problem;
                    }
                    thread::sleep(Duration::from_secs(2));
                }
            }
        }
    })
}

/// Sends one match to every display found within `wait` (or to `fixed`) and returns what each answered.
pub fn send_once(
    fixed: Option<IpAddr>,
    info: &MatchInfo,
    show_own_team: bool,
    lang: Lang,
    brightness: Brightness,
    wait: Duration,
) -> io::Result<Vec<(IpAddr, Ack)>> {
    let socket = bind(fixed)?;
    let mut devices: HashMap<IpAddr, Device> = HashMap::new();
    if let Some(addr) = fixed {
        devices.insert(addr, Device::new(true));
    }
    let deadline = Instant::now() + wait;
    let mut ask = Every::new(Duration::from_secs(1));
    while devices.is_empty() && Instant::now() < deadline {
        if ask.is_due() {
            probe(&socket);
            ask.done();
        }
        receive(&socket, &mut devices)?;
    }

    let frame = match_frame(info, show_own_team, lang, brightness).to_string();
    let mut answers = Vec::new();
    for addr in devices.keys() {
        socket.send_to(frame.as_bytes(), SocketAddr::new(*addr, FRAME_PORT))?;
    }
    let deadline = Instant::now() + Duration::from_millis(800);
    let mut buf = [0u8; 64];
    while answers.len() < devices.len() && Instant::now() < deadline {
        if let Ok((len, sender)) = socket.recv_from(&mut buf) {
            if let Some(ack) = Reply::parse(&buf[..len]) {
                if devices.contains_key(&sender.ip()) && !answers.iter().any(|(addr, _)| *addr == sender.ip()) {
                    answers.push((sender.ip(), ack.into()));
                }
            }
        }
    }
    for addr in devices.keys() {
        if !answers.iter().any(|(answered, _)| answered == addr) {
            answers.push((*addr, Ack::Silent));
        }
    }
    Ok(answers)
}
