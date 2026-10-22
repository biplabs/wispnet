//! Independent, retained QAD sessions for every explicitly configured observer.
use super::*;

impl Client {
    pub(super) async fn spawn_all_qad_probes(
        &mut self,
        state: &IfStateDetails,
        token: CancellationToken,
    ) -> Vec<ProbeReport> {
        let Some(client) = self.socket_state.quic_client.clone() else {
            return Vec::new();
        };
        let mut observers: BTreeMap<_, _> = self
            .relay_map
            .relays::<Vec<_>>()
            .into_iter()
            .map(|relay| (relay.url.clone(), relay))
            .collect();
        if let Some(observer) = self.quic_discovery_fallback.clone() {
            observers.entry(observer.url.clone()).or_insert(observer);
        }
        let mut reports = Vec::new();
        for (ipv6, connections) in [
            (false, &mut self.qad_conns.all_v4),
            (true, &mut self.qad_conns.all_v6),
        ] {
            let enabled = if ipv6 { state.have_v6 } else { state.have_v4 };
            connections.retain(|url, conn| {
                let keep =
                    enabled && observers.contains_key(url) && conn.conn.close_reason().is_none();
                if !keep {
                    conn.conn
                        .close(QUIC_ADDR_DISC_CLOSE_CODE, QUIC_ADDR_DISC_CLOSE_REASON);
                }
                keep
            });
            for conn in connections.values() {
                if let Some(mut report) = conn.observer.get() {
                    if let Some(rtt) = conn.conn.rtt(noq_proto::PathId::ZERO) {
                        report.latency = rtt;
                    }
                    reports.push(if ipv6 {
                        ProbeReport::QadIpv6(report)
                    } else {
                        ProbeReport::QadIpv4(report)
                    });
                }
            }
        }
        let mut tasks = JoinSet::new();
        for (url, relay) in observers.into_iter().take(6) {
            for ipv6 in [false, true] {
                let (enabled, cached) = if ipv6 {
                    (state.have_v6, self.qad_conns.all_v6.contains_key(&url))
                } else {
                    (state.have_v4, self.qad_conns.all_v4.contains_key(&url))
                };
                if !enabled || cached {
                    continue;
                }
                let relay = relay.clone();
                let client = client.clone();
                let resolver = self.socket_state.dns_resolver.clone();
                let cancel = token.child_token();
                tasks.spawn(async move {
                    let result = cancel
                        .run_until_cancelled(run_bounded_probe(
                            relay,
                            client,
                            resolver,
                            cancel.child_token(),
                            ipv6,
                        ))
                        .await;
                    (ipv6, result)
                });
            }
        }
        // Do not stop after the first successful observation: each relay may
        // expose a different mapping that needs to be advertised to peers.
        loop {
            tokio::select! {
                _ = token.cancelled() => break,
                result = tasks.join_next() => {
                    let Some(result) = result else { break; };
                    match result {
                        Ok((ipv6, Some(Ok((report, conn))))) => {
                            let url = report.relay.clone();
                            if ipv6 {
                                reports.push(ProbeReport::QadIpv6(report));
                                self.qad_conns.all_v6.insert(url, conn);
                            } else {
                                reports.push(ProbeReport::QadIpv4(report));
                                self.qad_conns.all_v4.insert(url, conn);
                            }
                        }
                        Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
                        _ => {},
                    }
                }
            }
        }
        reports
    }
}
