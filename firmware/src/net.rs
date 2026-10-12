//! WiFi station, DHCP and the UDP side of the display protocol (see `PROTOCOL.md`).

use core::cell::Cell;
use core::fmt::Write;

use display_protocol::board::WifiState;
use display_protocol::udp::{BEACON_PORT, Beacon, FRAME_PORT, is_probe, parse_datagram};
use embassy_executor::Spawner;
use embassy_futures::select::{Either3, select3};
use embassy_net::udp::{PacketMetadata, UdpSocket};
use embassy_net::{IpAddress, IpEndpoint, Runner, Stack, StackResources};
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::{Duration, Ticker, Timer};
use esp_hal::peripherals::WIFI;
use esp_hal::rng::Rng;
use esp_radio::wifi::sta::StationConfig;
use esp_radio::wifi::{AuthenticationMethodConfig, Config, ControllerConfig, Interface, WifiController};
use heapless::String;
use static_cell::StaticCell;

use crate::credentials::Credentials;
use crate::{FRAME, frame_reply};

const BEACON_INTERVAL: Duration = Duration::from_secs(2);
const RECONNECT_DELAY: Duration = Duration::from_secs(5);

static STATE: Mutex<CriticalSectionRawMutex, Cell<WifiState>> = Mutex::new(Cell::new(WifiState::Unset));

/// How the WiFi is doing, for the status screen and the answer to a status request.
pub fn wifi_state() -> WifiState {
    STATE.lock(Cell::get)
}

fn set_state(state: WifiState) {
    STATE.lock(|cell| cell.set(state));
}

/// Brings up WiFi and the UDP service with the credentials stored on the board. Without
/// any, the display works over USB only.
pub fn start(spawner: Spawner, wifi: WIFI<'static>, credentials: Option<Credentials>) {
    let Some(credentials) = credentials else {
        return;
    };
    set_state(WifiState::Off);
    let authentication = if credentials.password.is_empty() {
        AuthenticationMethodConfig::Open
    } else {
        match credentials.password.as_str().try_into() {
            Ok(password) => AuthenticationMethodConfig::Wpa2Personal(password),
            Err(_) => return,
        }
    };
    let Ok(ssid) = credentials.ssid.as_str().try_into() else {
        return;
    };
    let station = StationConfig::default().with_ssid(ssid).with_authentication(authentication);

    let config = ControllerConfig::default().with_initial_config(Config::Station(station));
    let Ok(controller) = WifiController::new(wifi, config) else {
        return;
    };
    let interface = Interface::station();
    let mac = interface.mac_address();

    let rng = Rng::new();
    let seed = u64::from(rng.random()) << 32 | u64::from(rng.random());
    static RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();
    let (stack, runner) = embassy_net::new(
        interface,
        embassy_net::Config::dhcpv4(Default::default()),
        RESOURCES.init(StackResources::new()),
        seed,
    );

    set_state(WifiState::Connecting);
    spawner.spawn(wifi_task(controller).expect("wifi task"));
    spawner.spawn(net_task(runner).expect("net task"));
    spawner.spawn(udp_task(stack, mac).expect("udp task"));
}

/// Keeps the station associated, retrying after a failure or a dropped connection.
#[embassy_executor::task]
async fn wifi_task(mut controller: WifiController<'static>) {
    loop {
        if controller.connect_async().await.is_ok() {
            let _ = controller.wait_for_disconnect_async().await;
        }
        Timer::after(RECONNECT_DELAY).await;
    }
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, Interface>) -> ! {
    runner.run().await
}

/// Announces the display on the LAN and accepts frames, one per datagram.
#[embassy_executor::task]
async fn udp_task(stack: Stack<'static>, mac: [u8; 6]) {
    let mut beacon: String<40> = String::new();
    let _ = write!(beacon, "{}", Beacon(mac));
    let broadcast = IpEndpoint::new(IpAddress::v4(255, 255, 255, 255), BEACON_PORT);

    let mut rx_meta = [PacketMetadata::EMPTY; 4];
    let mut rx_buffer = [0u8; 2048];
    let mut tx_meta = [PacketMetadata::EMPTY; 4];
    let mut tx_buffer = [0u8; 256];
    let mut datagram = [0u8; 1024];

    loop {
        stack.wait_config_up().await;
        if let Some(config) = stack.config_v4() {
            set_state(WifiState::Up(Some(config.address.address())));
        }

        let mut socket = UdpSocket::new(stack, &mut rx_meta, &mut rx_buffer, &mut tx_meta, &mut tx_buffer);
        if socket.bind(FRAME_PORT).is_ok() {
            let mut ticker = Ticker::every(BEACON_INTERVAL);
            loop {
                match select3(socket.recv_from(&mut datagram), ticker.next(), stack.wait_config_down()).await {
                    Either3::First(Ok((len, sender))) => {
                        if is_probe(&datagram[..len]) {
                            let _ = socket.send_to(beacon.as_bytes(), sender).await;
                        } else if let Some(frame) = parse_datagram(&datagram[..len]) {
                            FRAME.signal(frame);
                            let _ = socket.send_to(frame_reply(), sender).await;
                        }
                    }
                    Either3::First(Err(_)) => {}
                    Either3::Second(()) => {
                        let _ = socket.send_to(beacon.as_bytes(), broadcast).await;
                    }
                    Either3::Third(()) => break,
                }
            }
        }
        drop(socket);
        set_state(WifiState::Connecting);
        Timer::after(Duration::from_secs(1)).await;
    }
}
