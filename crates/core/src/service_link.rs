//! Stable service identity links. Links carry no credentials and grant no access.
use crate::{identity::EndpointId, protocol::valid_service_id};

#[derive(Debug, PartialEq)]
pub struct ServiceLink {
    pub host: EndpointId,
    pub service: String,
}
impl ServiceLink {
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        // Parse the exact grammar rather than accepting URL normalization,
        // credentials, ports, escaped separators, queries or extra path segments.
        let rest = value
            .strip_prefix("wispnet://")
            .ok_or("Invalid Wispnet link")?;
        let (host, service) = rest
            .split_once('/')
            .ok_or("Expected wispnet://<host-id>/<service-id>")?;
        if host.len() != 64
            || !host
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || !valid_service_id(service)
        {
            return Err("Invalid Wispnet host or service ID");
        }
        Ok(Self {
            host: host.parse().map_err(|_| "Invalid Wispnet host ID")?,
            service: service.into(),
        })
    }
    pub fn url(&self) -> String {
        format!("wispnet://{}/{}", self.host, self.service)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn links_are_exact_identity_references() {
        let host: EndpointId = "a".repeat(64).parse().unwrap();
        let link = ServiceLink {
            host,
            service: "gike".into(),
        };
        assert_eq!(ServiceLink::parse(&link.url()).unwrap(), link);
        for rest in [
            "gike/", "gike/api", "gike?x=1", "gike#x", "%67ike", "", "../gike",
        ] {
            assert!(ServiceLink::parse(&format!("wispnet://{host}/{rest}")).is_err());
        }
        for value in [
            format!("https://{host}/gike"),
            format!("wispnet://user@{host}/gike"),
            format!("wispnet://{host}:7842/gike"),
            "wispnet://short/gike".into(),
        ] {
            assert!(ServiceLink::parse(&value).is_err());
        }
    }
}
