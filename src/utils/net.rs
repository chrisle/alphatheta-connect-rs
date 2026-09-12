//! Network interface enumeration, the subset of Node's `os.networkInterfaces()`
//! this crate needs.

use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

/// An IPv4 network interface, the equivalent of Node's
/// `NetworkInterfaceInfoIPv4`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterfaceInfo {
    /// The OS interface name (`en0`, `eth0`, `Ethernet 3`).
    pub name: String,
    /// The IPv4 address of the host on this interface.
    pub address: Ipv4Addr,
    /// The subnet mask.
    pub netmask: Ipv4Addr,
    /// The interface MAC address, all zero when it cannot be determined.
    pub mac: [u8; 6],
    /// True for loopback interfaces.
    pub internal: bool,
}

impl InterfaceInfo {
    /// The prefix length of the netmask (`24` for `255.255.255.0`).
    pub fn prefix_len(&self) -> u32 {
        u32::from(self.netmask).count_ones()
    }

    /// The CIDR form, `10.0.0.5/24`.
    pub fn cidr(&self) -> String {
        format!("{}/{}", self.address, self.prefix_len())
    }

    /// True when `ip` is inside this interface's subnet.
    pub fn contains(&self, ip: Ipv4Addr) -> bool {
        let mask = u32::from(self.netmask);
        u32::from(ip) & mask == u32::from(self.address) & mask
    }

    /// The subnet broadcast address.
    pub fn broadcast(&self) -> Ipv4Addr {
        let mask = u32::from(self.netmask);
        Ipv4Addr::from(u32::from(self.address) | !mask)
    }

    /// The MAC as `aa:bb:cc:dd:ee:ff`.
    pub fn mac_string(&self) -> String {
        self.mac.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":")
    }
}

/// Every IPv4 interface on the host, loopbacks included.
pub fn network_interfaces() -> Vec<InterfaceInfo> {
    let Ok(addrs) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };

    addrs
        .into_iter()
        .filter_map(|iface| {
            let internal = iface.is_loopback();
            let if_addrs::IfAddr::V4(v4) = iface.addr else {
                return None;
            };
            let mac = mac_address::mac_address_by_name(&iface.name).ok().flatten().map(|m| m.bytes()).unwrap_or([0u8; 6]);
            Some(InterfaceInfo { name: iface.name, address: v4.ip, netmask: v4.netmask, mac, internal })
        })
        .collect()
}

/// All non-loopback IPv4 interfaces, keyed by name.
pub fn interfaces_by_name(name: &str) -> Vec<InterfaceInfo> {
    network_interfaces().into_iter().filter(|i| i.name == name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iface() -> InterfaceInfo {
        InterfaceInfo {
            name: "en0".into(),
            address: Ipv4Addr::new(10, 0, 0, 5),
            netmask: Ipv4Addr::new(255, 255, 255, 0),
            mac: [1, 2, 3, 4, 5, 6],
            internal: false,
        }
    }

    #[test]
    fn subnet_helpers() {
        let i = iface();
        assert_eq!(i.prefix_len(), 24);
        assert_eq!(i.cidr(), "10.0.0.5/24");
        assert!(i.contains(Ipv4Addr::new(10, 0, 0, 207)));
        assert!(!i.contains(Ipv4Addr::new(10, 0, 1, 1)));
        assert_eq!(i.broadcast(), Ipv4Addr::new(10, 0, 0, 255));
        assert_eq!(i.mac_string(), "01:02:03:04:05:06");
    }
}
