//! Cross-platform detection of AlphaTheta (Pioneer DJ) network interfaces.
//! Supports both USB-connected devices (XDJ-XZ, XDJ-AZ) and
//! Ethernet-connected devices.

use std::collections::{HashMap, HashSet};
use std::net::Ipv4Addr;
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::utils::net::{network_interfaces, InterfaceInfo};

/// Known MAC address prefixes (OUI) for AlphaTheta/Pioneer DJ devices. These
/// are used to identify devices on the network via the ARP cache.
const ALPHATHETA_MAC_PREFIXES: &[&str] = &[
    // AlphaTheta (XDJ-XZ, etc.)
    "c8:3d:fc", // Pioneer DJ
    "74:5e:1c", "ac:b5:7d", "b8:e8:56", // Realtek (used in some Pioneer devices)
    "00:e0:4c",
];

/// Check if a MAC address belongs to an AlphaTheta/Pioneer device.
pub fn is_alphatheta_mac(mac: &str) -> bool {
    let normalized = mac.to_lowercase();
    ALPHATHETA_MAC_PREFIXES.iter().any(|p| normalized.starts_with(p))
}

/// How a device is connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionType {
    Usb,
    Ethernet,
}

/// A network interface with an AlphaTheta device behind it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlphaThetaInterface {
    /// The interface name (e.g., "en15" on macOS, "Ethernet 3" on Windows).
    pub name: String,
    /// The MAC address of the interface.
    pub mac: String,
    /// The IPv4 address of the host on this interface.
    pub ipv4: Option<Ipv4Addr>,
    /// The interface details.
    pub info: Option<InterfaceInfo>,
    /// How the device is connected.
    pub connection_type: ConnectionType,
    /// IP addresses of AlphaTheta devices found on this interface.
    pub device_ips: Option<Vec<Ipv4Addr>>,
}

fn run(cmd: &str, args: &[&str], timeout: Duration) -> Option<String> {
    let mut child =
        Command::new(cmd).args(args).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().ok()?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return None,
        }
    }
    let output = child.wait_with_output().ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Find the network interface for an AlphaTheta device connected via USB or
/// Ethernet. Works on macOS, Windows, and Linux.
///
/// Detection methods (tried in order):
/// 1. USB: Looks for USB-connected devices (XDJ-XZ, XDJ-AZ) via system APIs
/// 2. Ethernet: Checks the ARP cache for known AlphaTheta/Pioneer MAC address prefixes
///
/// This is useful for passive mode to automatically detect the correct
/// interface for AlphaTheta devices like XDJ-XZ, XDJ-AZ, CDJ-3000, etc.
pub fn find_alphatheta_interface() -> Option<AlphaThetaInterface> {
    find_all_alphatheta_interfaces().into_iter().next()
}

/// Find ALL network interfaces with AlphaTheta/Pioneer DJ devices. Returns
/// all detected interfaces, with USB interfaces listed first.
pub fn find_all_alphatheta_interfaces() -> Vec<AlphaThetaInterface> {
    let mut results = Vec::new();
    let mut seen = HashSet::new();

    // USB detection first (more specific)
    let usb = if cfg!(target_os = "macos") {
        find_all_alphatheta_interfaces_macos()
    } else if cfg!(windows) {
        find_all_alphatheta_interfaces_windows()
    } else {
        Vec::new()
    };

    for iface in usb {
        seen.insert(iface.name.clone());
        results.push(iface);
    }

    // Ethernet detection via ARP cache (works on all platforms)
    for iface in find_all_alphatheta_via_ethernet() {
        // Avoid duplicates if same interface found via both methods
        if seen.insert(iface.name.clone()) {
            results.push(iface);
        }
    }

    results
}

fn ipv4_entry(name: &str, all: &[InterfaceInfo]) -> Option<InterfaceInfo> {
    all.iter().find(|i| i.name == name).cloned()
}

/// macOS: find AlphaTheta USB interfaces using `ioreg` and `networksetup` to
/// map them to interface names.
fn find_all_alphatheta_interfaces_macos() -> Vec<AlphaThetaInterface> {
    // Step 1: Check if an AlphaTheta USB device is connected
    let Some(ioreg) = run("ioreg", &["-p", "IOUSB", "-l", "-w", "0"], Duration::from_secs(5)) else {
        return Vec::new();
    };
    if !ioreg.to_lowercase().contains("alphatheta") {
        return Vec::new();
    }

    // Step 2: Get all hardware ports and find USB 10/100 LAN adapters
    let Some(ports) = run("networksetup", &["-listallhardwareports"], Duration::from_secs(5)) else {
        return Vec::new();
    };

    // Format:
    // Hardware Port: USB 10/100 LAN
    // Device: en15
    // Ethernet Address: c8:3d:fc:0a:58:54
    let mut usb_lan: Vec<(String, String)> = Vec::new();
    for block in ports.split("\n\n") {
        if block.contains("USB 10/100 LAN") || block.contains("USB 10_100 LAN") {
            let device = block.lines().find_map(|l| l.strip_prefix("Device:")).map(|s| s.trim().to_string());
            let mac = block.lines().find_map(|l| l.strip_prefix("Ethernet Address:")).map(|s| s.trim().to_string());
            if let (Some(device), Some(mac)) = (device, mac) {
                usb_lan.push((device, mac));
            }
        }
    }

    // Step 3: Match with active network interfaces
    let all = network_interfaces();
    usb_lan
        .into_iter()
        .filter_map(|(name, mac)| {
            let entry = ipv4_entry(&name, &all)?;
            Some(AlphaThetaInterface {
                name,
                mac,
                ipv4: Some(entry.address),
                info: Some(entry),
                connection_type: ConnectionType::Usb,
                device_ips: None,
            })
        })
        .collect()
}

/// Windows: find AlphaTheta USB network adapters via PowerShell.
fn find_all_alphatheta_interfaces_windows() -> Vec<AlphaThetaInterface> {
    let ps = "Get-NetAdapter | Where-Object { $_.InterfaceDescription -like '*AlphaTheta*' -or $_.InterfaceDescription -like '*Pioneer*' -or $_.InterfaceDescription -like '*USB 10/100 LAN*' } | Select-Object -Property Name, MacAddress, InterfaceDescription | ForEach-Object { $_.Name + '|' + $_.MacAddress }";
    let Some(output) = run("powershell", &["-Command", ps], Duration::from_secs(10)) else {
        return Vec::new();
    };

    let all = network_interfaces();
    let mut results = Vec::new();
    for line in output.lines() {
        let mut parts = line.split('|');
        let (Some(name), Some(mac)) = (parts.next(), parts.next()) else {
            continue;
        };
        let target_mac = mac.replace('-', ":").to_lowercase();

        // Windows interface names may differ between tools; try matching by
        // MAC address first, then by name.
        let matched = all.iter().find(|i| i.mac_string() == target_mac).or_else(|| all.iter().find(|i| i.name == name));
        if let Some(entry) = matched {
            results.push(AlphaThetaInterface {
                name: entry.name.clone(),
                mac: entry.mac_string(),
                ipv4: Some(entry.address),
                info: Some(entry.clone()),
                connection_type: ConnectionType::Usb,
                device_ips: None,
            });
        }
    }
    results
}

/// Get the device IP address from a link-local interface.
///
/// When connected via USB, the device typically uses a link-local address in
/// the 169.254.x.x range. Without the ARP cache or actual discovery we cannot
/// know it for sure, so this always returns `None` for now, as upstream does.
pub fn infer_device_ip_from_host(host_ip: Ipv4Addr) -> Option<Ipv4Addr> {
    let _ = host_ip.is_link_local();
    None
}

fn parse_ip_in_parens(line: &str) -> Option<Ipv4Addr> {
    let start = line.find('(')? + 1;
    let end = line[start..].find(')')? + start;
    line[start..end].parse().ok()
}

fn parse_after(line: &str, marker: &str) -> Option<String> {
    let at = line.find(marker)? + marker.len();
    line[at..].split_whitespace().next().map(|s| s.to_string())
}

/// Try to find device IPs by checking the ARP cache. Works on macOS and Linux.
pub fn get_arp_cache_for_interface(interface_name: &str) -> Vec<Ipv4Addr> {
    if !(cfg!(target_os = "macos") || cfg!(target_os = "linux")) {
        return Vec::new();
    }
    let Some(arp) = run("arp", &["-an"], Duration::from_secs(5)) else {
        return Vec::new();
    };
    // macOS format: ? (169.254.88.83) at c8:3d:fc:a:58:55 on en15 ifscope [ethernet]
    // Linux format: ? (169.254.88.83) at c8:3d:fc:a:58:55 [ether] on en15
    arp.lines().filter(|l| l.contains(interface_name)).filter_map(parse_ip_in_parens).collect()
}

/// Parse the ARP cache and return entries with AlphaTheta MAC addresses,
/// keyed by interface name (or by interface IP on Windows).
fn get_alphatheta_arp_entries() -> HashMap<String, Vec<(Ipv4Addr, String)>> {
    let mut result: HashMap<String, Vec<(Ipv4Addr, String)>> = HashMap::new();

    if cfg!(target_os = "macos") || cfg!(target_os = "linux") {
        let Some(arp) = run("arp", &["-an"], Duration::from_secs(5)) else {
            return result;
        };
        for line in arp.lines() {
            let (Some(ip), Some(mac), Some(iface)) =
                (parse_ip_in_parens(line), parse_after(line, " at "), parse_after(line, " on "))
            else {
                continue;
            };
            if is_alphatheta_mac(&mac) {
                result.entry(iface).or_default().push((ip, mac));
            }
        }
    } else if cfg!(windows) {
        let Some(arp) = run("arp", &["-a"], Duration::from_secs(5)) else {
            return result;
        };
        let mut current = String::new();
        for line in arp.lines() {
            // Interface header: Interface: 192.168.1.100 --- 0x5
            if let Some(rest) = line.trim().strip_prefix("Interface:") {
                current = rest.split_whitespace().next().unwrap_or("").to_string();
                continue;
            }
            // Entry: 192.168.1.1     00-11-22-33-44-55     dynamic
            let mut parts = line.split_whitespace();
            let (Some(ip), Some(mac)) = (parts.next().and_then(|s| s.parse::<Ipv4Addr>().ok()), parts.next()) else {
                continue;
            };
            let mac = mac.replace('-', ":");
            if !current.is_empty() && is_alphatheta_mac(&mac) {
                result.entry(current.clone()).or_default().push((ip, mac));
            }
        }
    }

    result
}

/// Find ALL AlphaTheta devices connected via Ethernet by checking the ARP
/// cache for known MAC address prefixes.
fn find_all_alphatheta_via_ethernet() -> Vec<AlphaThetaInterface> {
    let entries = get_alphatheta_arp_entries();
    if entries.is_empty() {
        return Vec::new();
    }

    let all = network_interfaces();
    let mut results = Vec::new();

    for (arp_iface, devices) in entries {
        // On Windows, ARP uses IP addresses as interface identifiers; find
        // the matching interface by address.
        let entry = ipv4_entry(&arp_iface, &all).or_else(|| {
            let ip: Ipv4Addr = arp_iface.parse().ok()?;
            all.iter().find(|i| i.address == ip).cloned()
        });
        let Some(entry) = entry else {
            continue;
        };

        results.push(AlphaThetaInterface {
            name: entry.name.clone(),
            mac: entry.mac_string(),
            ipv4: Some(entry.address),
            info: Some(entry),
            connection_type: ConnectionType::Ethernet,
            device_ips: Some(devices.into_iter().map(|(ip, _)| ip).collect()),
        });
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_alphatheta_macs() {
        assert!(is_alphatheta_mac("C8:3D:FC:0A:58:54"));
        assert!(is_alphatheta_mac("74:5e:1c:01:02:03"));
        assert!(!is_alphatheta_mac("00:11:22:33:44:55"));
    }

    #[test]
    fn parses_arp_lines() {
        let mac_line = "? (169.254.88.83) at c8:3d:fc:a:58:55 on en15 ifscope [ethernet]";
        assert_eq!(parse_ip_in_parens(mac_line), Some(Ipv4Addr::new(169, 254, 88, 83)));
        assert_eq!(parse_after(mac_line, " at ").as_deref(), Some("c8:3d:fc:a:58:55"));
        assert_eq!(parse_after(mac_line, " on ").as_deref(), Some("en15"));
        let linux_line = "? (169.254.88.83) at c8:3d:fc:a:58:55 [ether] on eth0";
        assert_eq!(parse_after(linux_line, " on ").as_deref(), Some("eth0"));
    }
}
