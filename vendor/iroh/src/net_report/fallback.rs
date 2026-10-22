//! Ordered address-discovery fallback policy, independent of QUIC mechanics.
use std::{
    future::Future,
    net::{IpAddr, SocketAddr},
};

/// A usable primary result never constructs or polls the outside request.
pub(super) async fn choose<T, E, P, F, O, U>(primary: P, outside: F, usable: U) -> Result<T, E>
where
    P: Future<Output = Result<T, E>>,
    F: FnOnce() -> O,
    O: Future<Output = Result<T, E>>,
    U: Fn(&T) -> bool,
{
    let first = primary.await;
    if first.as_ref().is_ok_and(&usable) {
        return first;
    }
    let second = outside().await;
    if second.as_ref().is_ok_and(usable) || first.is_err() {
        second
    } else {
        first
    }
}

pub(super) fn public_address(address: SocketAddr) -> bool {
    if address.port() == 0 {
        return false;
    }
    match address.ip() {
        IpAddr::V4(ip) => {
            let [a, b, c, d] = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_documentation()
                || a == 0
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 198 && (b == 18 || b == 19))
                || (a == 192 && b == 0 && c == 0 && d != 9 && d != 10)
                || (a == 192 && b == 88 && c == 99))
        }
        IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                return public_address(SocketAddr::new(v4.into(), address.port()));
            }
            let s = ip.segments();
            s[0] & 0xe000 == 0x2000
                && !(s[0] == 0x2001 && (s[1] == 0xdb8 || (s[1] == 2 && s[2] == 0)))
                && !(s[0] == 0x3fff && s[1] & 0xf000 == 0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    #[tokio::test]
    async fn usable_primary_never_contacts_outside() {
        let called = Cell::new(false);
        let result = choose(
            async { Ok::<_, ()>("162.156.47.159:10000".parse::<SocketAddr>().unwrap()) },
            || {
                called.set(true);
                async { panic!("outside request must not be constructed") }
            },
            |a| public_address(*a),
        )
        .await
        .unwrap();
        assert_eq!(result.port(), 10000);
        assert!(!called.get());
    }
    #[tokio::test]
    async fn private_primary_then_outside_preserves_actual_udp_port() {
        let order = Cell::new(0);
        let result = choose(
            async {
                order.set(1);
                Ok::<_, ()>("192.168.1.1:20000".parse::<SocketAddr>().unwrap())
            },
            || {
                assert_eq!(order.get(), 1);
                order.set(2);
                async { Ok("162.156.47.159:30000".parse().unwrap()) }
            },
            |a| public_address(*a),
        )
        .await
        .unwrap();
        assert_eq!(order.get(), 2);
        assert_eq!(result.port(), 30000);
    }
    #[tokio::test]
    async fn primary_failure_uses_outside_and_outside_failure_retains_lan_result() {
        let good = "162.156.47.159:10000".parse::<SocketAddr>().unwrap();
        assert_eq!(
            choose(
                async { Err(()) },
                || async { Ok(good) },
                |a| public_address(*a)
            )
            .await,
            Ok(good)
        );
        let lan = "192.168.1.1:10000".parse::<SocketAddr>().unwrap();
        assert_eq!(
            choose(
                async { Ok(lan) },
                || async { Err(()) },
                |a| public_address(*a)
            )
            .await,
            Ok(lan)
        );
        assert!(
            choose::<SocketAddr, (), _, _, _, _>(
                async { Err(()) },
                || async { Err(()) },
                |a| public_address(*a)
            )
            .await
            .is_err()
        );
    }
    #[test]
    fn non_public_observations_require_fallback() {
        for ip in [
            "162.156.47.159:0",
            "10.1.2.3:1",
            "100.64.0.1:1",
            "127.0.0.1:1",
            "169.254.1.1:1",
            "[::1]:1",
            "[fc00::1]:1",
        ] {
            assert!(!public_address(ip.parse().unwrap()), "{ip}");
        }
    }
}
