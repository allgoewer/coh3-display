//! Finds displays on the LAN by their beacon and sends them frames over UDP.

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::thread;
use std::time::{Duration, Instant};

use super::{Ack, DISABLED_POLL, LinkContext, RESEND, Stop, WifiLink, encode, frame_for};
use crate::applog::log;
use crate::model::MatchInfo;

/// Displays broadcast their beacon to this port.
pub const BEACON_PORT: u16 = 42424;
/// Displays receive frames on this port.
pub const FRAME_PORT: u16 = 42425;
const BEACON_PREFIX: &str = "COH3DISPLAY 1 ";
/// Broadcast by the PC; a display answers it with its beacon, sent straight back. Unlike the
/// display's own broadcast, that answer gets through a firewall that blocks unsolicited traffic.
const PROBE: &[u8] = b"COH3DISPLAY? 1\n";
const PROBE_INTERVAL: Duration = Duration::from_secs(2);
/// A display that stopped sending beacons for this long is forgotten.
const STALE: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(200);

/// Returns the display's ID (its MAC address) if `data` is a beacon.
pub fn parse_beacon(data: &[u8]) -> Option<&str> {
    let id = std::str::from_utf8(data).ok()?.trim_end().strip_prefix(BEACON_PREFIX)?;
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_hexdigit())).then_some(id)
}

fn parse_ack(data: &[u8]) -> Option<Ack> {
    if data.starts_with(b"OK") {
        Some(Ack::Shown)
    } else if data.starts_with(b"ERR") {
        Some(Ack::NoOled)
    } else {
        None
    }
}

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
        } else if let (Some(ack), Some(device)) = (parse_ack(data), devices.get_mut(&sender.ip())) {
            device.awaiting_ack = false;
            device.report(sender.ip(), ack);
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
    let mut last_sent: Option<Instant> = None;
    let mut last_probe: Option<Instant> = None;
    loop {
        let Ok(match_changed) = ctx.matches.has_changed() else {
            return Ok(Stop::Closed);
        };
        let settings_changed = ctx.settings.has_changed().unwrap_or(false);
        let settings = *ctx.settings.borrow_and_update();
        if !settings.wifi {
            return Ok(Stop::Disabled);
        }
        if last_probe.is_none_or(|at| at.elapsed() >= PROBE_INTERVAL) {
            probe(socket);
            last_probe = Some(Instant::now());
        }
        let found_new = receive(socket, &mut devices)?;
        devices.retain(|addr, device| {
            let keep = device.fixed || device.last_seen.elapsed() < STALE;
            if !keep {
                log(format!("Display at {addr} is gone"));
            }
            keep
        });

        if match_changed || settings_changed || found_new || last_sent.is_none_or(|at| at.elapsed() >= RESEND) {
            let frame = frame_for(ctx.matches.borrow_and_update().as_ref(), &settings);
            send(socket, &mut devices, &frame);
            last_sent = Some(Instant::now());
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
    wait: Duration,
) -> io::Result<Vec<(IpAddr, Ack)>> {
    let socket = bind(fixed)?;
    let mut devices: HashMap<IpAddr, Device> = HashMap::new();
    if let Some(addr) = fixed {
        devices.insert(addr, Device::new(true));
    }
    let deadline = Instant::now() + wait;
    let mut last_probe: Option<Instant> = None;
    while devices.is_empty() && Instant::now() < deadline {
        if last_probe.is_none_or(|at| at.elapsed() >= Duration::from_secs(1)) {
            probe(&socket);
            last_probe = Some(Instant::now());
        }
        receive(&socket, &mut devices)?;
    }

    let frame = encode(info, show_own_team);
    let mut answers = Vec::new();
    for addr in devices.keys() {
        socket.send_to(frame.as_bytes(), SocketAddr::new(*addr, FRAME_PORT))?;
    }
    let deadline = Instant::now() + Duration::from_millis(800);
    let mut buf = [0u8; 64];
    while answers.len() < devices.len() && Instant::now() < deadline {
        if let Ok((len, sender)) = socket.recv_from(&mut buf) {
            if let Some(ack) = parse_ack(&buf[..len]) {
                if devices.contains_key(&sender.ip()) && !answers.iter().any(|(addr, _)| *addr == sender.ip()) {
                    answers.push((sender.ip(), ack));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_beacons() {
        assert_eq!(parse_beacon(b"COH3DISPLAY 1 f0f5bd2d1b88\n"), Some("f0f5bd2d1b88"));
        assert_eq!(parse_beacon(b"COH3DISPLAY 1 \n"), None);
        assert_eq!(parse_beacon(b"COH3DISPLAY 2 f0f5bd2d1b88\n"), None);
        assert_eq!(parse_beacon(b"OK\n"), None);
    }

    #[test]
    fn recognises_acks() {
        assert_eq!(parse_ack(b"OK\n"), Some(Ack::Shown));
        assert_eq!(parse_ack(b"ERR display\n"), Some(Ack::NoOled));
        assert_eq!(parse_ack(b"COH3DISPLAY 1 f0f5bd2d1b88\n"), None);
    }
}
