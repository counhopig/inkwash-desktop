//! Tauri command for scanning nearby 2.4 GHz Wi-Fi networks from the PC
//! (not the device - the firmware has no scan command). Each platform uses
//! the most reliable local mechanism:
//!
//! * macOS - CoreWLAN via `objc2-core-wlan` (`scanForNetworksWithName:`).
//!   Requires Location Services authorization for SSID visibility (see the
//!   `NSLocationWhenInUseUsageDescription` entry in `tauri.conf.json`).
//! * Windows - `netsh wlan show networks mode=bssid` output parsing.
//! * Linux - `nmcli -t -e no -f SSID,CHAN,SIGNAL,SECURITY dev wifi list`
//!   output parsing (NetworkManager).
//!
//! Every platform returns the same `WifiNetwork` shape so the Vue frontend
//! has one list to render. Only 2.4 GHz APs (channels 1-14) are returned -
//! the ESP32-S3 only supports 2.4 GHz, so 5 GHz results are noise.

use serde::Serialize;
use tauri::State;
use ts_rs::TS;

use crate::desktop::SharedState;
use crate::error::AppError;

/// One nearby access point, already filtered to the 2.4 GHz band.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../src-ui/lib/generated/")]
#[serde(rename_all = "camelCase")]
pub struct WifiNetwork {
    pub ssid: String,
    pub channel: u16,
    /// Signal strength as a 0-100 percentage (approximated from dBm on
    /// macOS, reported directly by nmcli / netsh elsewhere).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub signal: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub security: Option<String>,
}

/// The 2.4 GHz band covers channels 1-14 (14 is only used in Japan).
fn is_24ghz_channel(channel: u16) -> bool {
    (1..=14).contains(&channel)
}

/// Maps an RSSI value in dBm to a rough 0-100 percentage. A value of 0
/// means "no measurement" per CoreWLAN's docs and maps to `None`.
#[cfg(target_os = "macos")]
fn signal_from_rssi(rssi: isize) -> Option<u8> {
    if rssi == 0 {
        return None;
    }
    let clamped = rssi.clamp(-100, -30);
    Some(((clamped + 100) * 100 / 70) as u8)
}

#[tauri::command]
pub async fn scan_wifi_networks(
    state: State<'_, SharedState>,
) -> Result<Vec<WifiNetwork>, AppError> {
    let shared = state.inner().clone();
    let networks = tauri::async_runtime::spawn_blocking(scan_24ghz)
        .await
        .map_err(|e| AppError::internal(format!("wifi scan task: {e}")))??;
    shared.logs.info(
        "wifi",
        format!("scanned {} 2.4GHz network(s)", networks.len()),
    );
    Ok(networks)
}

/// Platform dispatch for the actual scan. Runs on a blocking thread because
/// every backend can take a few seconds.
#[cfg(target_os = "macos")]
fn scan_24ghz() -> Result<Vec<WifiNetwork>, AppError> {
    use objc2_core_wlan::CWWiFiClient;

    // Safety: the CoreWLAN API is a plain Obj-C call on the current thread.
    // `CWWiFiClient::sharedWiFiClient()` returns a process singleton and
    // `scanForNetworksWithName:` blocks until the scan completes. CWNetwork
    // objects are read-only snapshots, safe to consume locally.
    unsafe {
        let client = CWWiFiClient::sharedWiFiClient();
        let Some(interface) = client.interface() else {
            return Err(AppError::internal(
                "no Wi-Fi interface found - is Wi-Fi turned on?",
            ));
        };

        // `include_hidden` so hidden networks still show up as candidates.
        let networks = interface
            .scanForNetworksWithName_includeHidden_error(None, true)
            .map_err(|e| AppError::internal(format!("CoreWLAN scan failed: {e}")))?;

        let mut result: Vec<WifiNetwork> = Vec::new();
        for net in &networks {
            // SSID is nil when Location Services is not authorized for this
            // app - surface that as a targeted error instead of an empty list.
            let Some(ssid) = net.ssid().map(|s| s.to_string()) else {
                return Err(AppError::internal(
                    "Wi-Fi scan returned no SSIDs - grant Location Services access to Inkwash Desktop in System Settings, then retry",
                ));
            };
            let channel = net
                .wlanChannel()
                .map(|c| c.channelNumber() as u16)
                .unwrap_or(0);
            if !is_24ghz_channel(channel) {
                continue;
            }
            result.push(WifiNetwork {
                ssid,
                channel,
                signal: signal_from_rssi(net.rssiValue()),
                security: None,
            });
        }
        Ok(result)
    }
}

/// Parses `netsh wlan show networks mode=bssid` output. The field names are
/// localized, so both English and Simplified-Chinese keys are matched
/// (output is decoded as GBK on zh-CN systems).
#[cfg(target_os = "windows")]
fn scan_24ghz() -> Result<Vec<WifiNetwork>, AppError> {
    let out = std::process::Command::new("netsh")
        .args(["wlan", "show", "networks", "mode=bssid"])
        .output()
        .map_err(|e| AppError::internal(format!("failed to run netsh: {e}")))?;
    if !out.status.success() {
        return Err(AppError::internal(
            "netsh wlan show networks failed - is Wi-Fi turned on?",
        ));
    }
    let text = if let Ok(s) = std::str::from_utf8(&out.stdout) {
        s.to_string()
    } else {
        let (decoded, _, _) = encoding_rs::GBK.decode(&out.stdout);
        decoded.into_owned()
    };
    Ok(parse_netsh(&text))
}

#[cfg(target_os = "windows")]
fn parse_netsh(text: &str) -> Vec<WifiNetwork> {
    let mut networks: Vec<WifiNetwork> = Vec::new();
    let mut current: Option<WifiNetwork> = None;

    for raw_line in text.lines() {
        let line = raw_line.trim();
        // A new AP block starts at "SSID <n> : <name>". Take everything
        // after the first ':' so SSIDs that themselves contain ':' survive.
        if let Some(rest) = line.strip_prefix("SSID ") {
            if let Some(name) = rest.split_once(':').map(|(_, name)| name.trim()) {
                if !name.is_empty() {
                    if let Some(n) = current.take() {
                        if is_24ghz_channel(n.channel) {
                            networks.push(n);
                        }
                    }
                    current = Some(WifiNetwork {
                        ssid: name.to_string(),
                        channel: 0,
                        signal: None,
                        security: None,
                    });
                    continue;
                }
            }
        }
        let Some(net) = current.as_mut() else {
            continue;
        };
        let lower = line.to_lowercase();
        if lower.contains("channel") || lower.contains("信道") {
            if let Some(ch) = line
                .rsplit(':')
                .next()
                .and_then(|v| v.trim().parse::<u16>().ok())
            {
                net.channel = ch;
            }
        } else if lower.contains("signal") || lower.contains("信号") {
            net.signal = line
                .rsplit(':')
                .next()
                .and_then(|v| v.trim().trim_end_matches('%').parse::<u8>().ok());
        } else if lower.contains("authentication") || lower.contains("身份验证") {
            if let Some(sec) = line.rsplit(':').next() {
                let sec = sec.trim();
                if !sec.is_empty() && sec != "Open" && sec != "开放式" {
                    net.security = Some(sec.to_string());
                }
            }
        }
    }
    if let Some(n) = current {
        if is_24ghz_channel(n.channel) {
            networks.push(n);
        }
    }
    networks.sort_by_key(|n| std::cmp::Reverse(n.signal));
    networks
}

/// Runs `nmcli` (NetworkManager) and parses its escaped TSV output.
#[cfg(target_os = "linux")]
fn scan_24ghz() -> Result<Vec<WifiNetwork>, AppError> {
    let out = std::process::Command::new("nmcli")
        .args([
            "-t",
            "-e",
            "no",
            "-f",
            "SSID,CHAN,SIGNAL,SECURITY",
            "dev",
            "wifi",
            "list",
        ])
        .output()
        .map_err(|e| AppError::internal(format!("failed to run nmcli: {e}")))?;
    if !out.status.success() {
        return Err(AppError::internal(
            "nmcli failed - is NetworkManager running?",
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(parse_nmcli(&text))
}

/// Parses `nmcli -t -e no -f SSID,CHAN,SIGNAL,SECURITY dev wifi list` output:
/// one `SSID:CHAN:SIGNAL:SECURITY` row per line (first row is the header).
#[cfg(target_os = "linux")]
fn parse_nmcli(text: &str) -> Vec<WifiNetwork> {
    let mut networks: Vec<WifiNetwork> = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        if idx == 0 {
            continue; // header row
        }
        let parts: Vec<&str> = line.splitn(4, ':').collect();
        if parts.len() < 4 {
            continue;
        }
        let ssid = parts[0].trim();
        if ssid.is_empty() {
            continue; // hidden network, no SSID to select
        }
        let Ok(channel) = parts[1].trim().parse::<u16>() else {
            continue;
        };
        if !is_24ghz_channel(channel) {
            continue;
        }
        let signal = parts[2].trim().parse::<u8>().ok();
        let security = match parts[3].trim() {
            "" | "--" => None,
            s => Some(s.to_string()),
        };
        networks.push(WifiNetwork {
            ssid: ssid.to_string(),
            channel,
            signal,
            security,
        });
    }
    networks.sort_by_key(|n| std::cmp::Reverse(n.signal));
    networks
}

/// No supported scanning backend on other platforms.
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn scan_24ghz() -> Result<Vec<WifiNetwork>, AppError> {
    Err(AppError::internal(
        "Wi-Fi scanning is not supported on this platform",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_24ghz_band() {
        assert!(is_24ghz_channel(1));
        assert!(is_24ghz_channel(6));
        assert!(is_24ghz_channel(13));
        assert!(!is_24ghz_channel(0));
        assert!(!is_24ghz_channel(14 + 1));
        assert!(!is_24ghz_channel(36));
        assert!(!is_24ghz_channel(149));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn rssi_maps_to_percentage() {
        assert_eq!(signal_from_rssi(-30), Some(100));
        assert_eq!(signal_from_rssi(-65), Some(50));
        assert_eq!(signal_from_rssi(-100), Some(0));
        assert_eq!(signal_from_rssi(0), None);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn parses_netsh_output_and_filters_24ghz() {
        let sample = r#"Interface name : Wi-Fi
There is currently 1 network visible.

SSID 1 : MyNetwork
    Network type : Infrastructure
    Authentication : WPA2-Personal
    Encryption : CCMP
    BSSID 1 : 12:34:56:78:9a:bc
         Signal : 80%
         Radio type : 802.11n
         Channel : 6

SSID 2 : Office5G
    Network type : Infrastructure
    Authentication : WPA2-Personal
    Encryption : CCMP
    BSSID 1 : 12:34:56:78:9a:bd
         Signal : 90%
         Radio type : 802.11ac
         Channel : 36
"#;
        let nets = parse_netsh(sample);
        assert_eq!(nets.len(), 1);
        assert_eq!(nets[0].ssid, "MyNetwork");
        assert_eq!(nets[0].channel, 6);
        assert_eq!(nets[0].signal, Some(80));
        assert_eq!(nets[0].security.as_deref(), Some("WPA2-Personal"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_nmcli_output_and_filters_24ghz() {
        let sample =
            "SSID:CHAN:SIGNAL:SECURITY\nMyNetwork:6:80:WPA2\nOffice5G:36:90:WPA2\nOpenNet:1:50:\n";
        let nets = parse_nmcli(sample);
        assert_eq!(nets.len(), 2);
        assert_eq!(nets[0].ssid, "MyNetwork");
        assert_eq!(nets[0].channel, 6);
        assert_eq!(nets[0].signal, Some(80));
        assert_eq!(nets[0].security.as_deref(), Some("WPA2"));
        assert_eq!(nets[1].ssid, "OpenNet");
        assert!(nets[1].security.is_none());
    }
}
