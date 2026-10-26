//! OPC UA client and plant server forms.

use vt_protocol::{
    OpcuaAuth, OpcuaClientSpec, OpcuaNode, OpcuaSecurityMode, OpcuaSecurityPolicy, OpcuaServerSpec,
    opcua_host_port,
};

use super::{Field, FormError, error, name, number};

/// How the client signs in, chosen in the form; each keeps its own fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpcuaAuthKind {
    Anonymous,
    UserName,
    Certificate,
}

impl OpcuaAuthKind {
    pub const ALL: [OpcuaAuthKind; 3] = [Self::Anonymous, Self::UserName, Self::Certificate];

    pub fn name(self) -> &'static str {
        match self {
            Self::Anonymous => "Anonymous",
            Self::UserName => "User name",
            Self::Certificate => "Certificate",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpcuaClientForm {
    pub name: String,
    pub endpoint: String,
    pub policy: OpcuaSecurityPolicy,
    pub mode: OpcuaSecurityMode,
    pub auth: OpcuaAuthKind,
    pub username: String,
    pub password: String,
    pub certificate: String,
    pub private_key: String,
    pub publishing_ms: String,
    pub sampling_ms: String,
    /// Not typed in the form: added from the address-space browser, removed in the editor.
    pub nodes: Vec<OpcuaNode>,
}

impl OpcuaClientForm {
    pub fn from_spec(name: &str, spec: &OpcuaClientSpec) -> Self {
        let (auth, username, password, certificate, private_key) = match &spec.auth {
            OpcuaAuth::Anonymous => (
                OpcuaAuthKind::Anonymous,
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ),
            OpcuaAuth::UserName { username, password } => (
                OpcuaAuthKind::UserName,
                username.clone(),
                password.clone(),
                String::new(),
                String::new(),
            ),
            OpcuaAuth::Certificate {
                certificate,
                private_key,
            } => (
                OpcuaAuthKind::Certificate,
                String::new(),
                String::new(),
                certificate.clone(),
                private_key.clone(),
            ),
        };
        Self {
            name: name.to_owned(),
            endpoint: spec.endpoint.clone(),
            policy: spec.policy,
            mode: spec.mode,
            auth,
            username,
            password,
            certificate,
            private_key,
            publishing_ms: spec.publishing_ms.to_string(),
            sampling_ms: spec.sampling_ms.to_string(),
            nodes: spec.nodes.clone(),
        }
    }

    pub fn build(&self) -> Result<(String, OpcuaClientSpec), FormError> {
        let name = name(&self.name)?;
        let endpoint = self.endpoint.trim();
        if opcua_host_port(endpoint).is_none() {
            return Err(error(
                Field::Endpoint,
                "Enter the server's endpoint, e.g. opc.tcp://192.168.1.20:4840",
            ));
        }
        match (self.policy, self.mode) {
            (OpcuaSecurityPolicy::None, OpcuaSecurityMode::None) => {}
            (OpcuaSecurityPolicy::None, _) => {
                return Err(error(
                    Field::Security,
                    "Messages can only be signed with a security policy: pick one, or mode None",
                ));
            }
            (_, OpcuaSecurityMode::None) => {
                return Err(error(
                    Field::Security,
                    "A security policy needs Sign or Sign & Encrypt",
                ));
            }
            _ => {}
        }
        let auth = match self.auth {
            OpcuaAuthKind::Anonymous => OpcuaAuth::Anonymous,
            OpcuaAuthKind::UserName => {
                let username = self.username.trim();
                if username.is_empty() {
                    return Err(error(Field::Username, "Enter the user name"));
                }
                OpcuaAuth::UserName {
                    username: username.to_owned(),
                    password: self.password.clone(),
                }
            }
            OpcuaAuthKind::Certificate => {
                let certificate = self.certificate.trim();
                if certificate.is_empty() {
                    return Err(error(
                        Field::Certificate,
                        "Choose the user certificate (DER or PEM)",
                    ));
                }
                let private_key = self.private_key.trim();
                if private_key.is_empty() {
                    return Err(error(
                        Field::PrivateKey,
                        "Choose the certificate's private key (PEM)",
                    ));
                }
                OpcuaAuth::Certificate {
                    certificate: certificate.to_owned(),
                    private_key: private_key.to_owned(),
                }
            }
        };
        let publishing_ms = number(
            &self.publishing_ms,
            Field::PublishingMs,
            "Publishing interval",
            50,
            3_600_000,
        )?;
        let sampling_ms = number(
            &self.sampling_ms,
            Field::SamplingMs,
            "Sampling interval",
            0,
            3_600_000,
        )?;
        Ok((
            name,
            OpcuaClientSpec {
                endpoint: endpoint.to_owned(),
                policy: self.policy,
                mode: self.mode,
                auth,
                publishing_ms,
                sampling_ms,
                nodes: self.nodes.clone(),
            },
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpcuaServerForm {
    pub name: String,
    pub bind: String,
    pub port: String,
    pub anonymous: bool,
    pub username: String,
    pub password: String,
}

impl OpcuaServerForm {
    pub fn from_spec(name: &str, spec: &OpcuaServerSpec) -> Self {
        Self {
            name: name.to_owned(),
            bind: spec.bind.clone(),
            port: spec.port.to_string(),
            anonymous: spec.anonymous,
            username: spec.username.clone(),
            password: spec.password.clone(),
        }
    }

    pub fn build(&self) -> Result<(String, OpcuaServerSpec), FormError> {
        let name = name(&self.name)?;
        let bind = self.bind.trim();
        let bare = bind
            .strip_prefix('[')
            .and_then(|inner| inner.strip_suffix(']'))
            .unwrap_or(bind);
        let Ok(ip) = bare.parse::<std::net::IpAddr>() else {
            return Err(error(
                Field::Bind,
                "Listen on an IP address, e.g. 127.0.0.1, 0.0.0.0 or ::",
            ));
        };
        let port = number(&self.port, Field::Port, "Port", 1, u16::MAX)?;
        let username = self.username.trim();
        if !username.is_empty() && self.password.is_empty() {
            return Err(error(Field::Password, "Give the user a password"));
        }
        Ok((
            name,
            OpcuaServerSpec {
                bind: ip.to_string(),
                port,
                anonymous: self.anonymous,
                username: username.to_owned(),
                password: if username.is_empty() {
                    String::new()
                } else {
                    self.password.clone()
                },
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> OpcuaClientForm {
        OpcuaClientForm::from_spec("Plant", &OpcuaClientSpec::default())
    }

    #[test]
    fn default_forms_round_trip_to_the_same_spec() {
        let (name, spec) = client().build().unwrap();
        assert_eq!(name, "Plant");
        assert_eq!(spec, OpcuaClientSpec::default());
        let server = OpcuaServerForm::from_spec("Sim", &OpcuaServerSpec::default());
        assert_eq!(server.build().unwrap().1, OpcuaServerSpec::default());
        // Saved nodes and credentials survive an edit.
        let saved = OpcuaClientSpec {
            auth: OpcuaAuth::Certificate {
                certificate: "/certs/op.der".into(),
                private_key: "/certs/op.pem".into(),
            },
            policy: OpcuaSecurityPolicy::Basic256Sha256,
            mode: OpcuaSecurityMode::Sign,
            nodes: vec![OpcuaNode::new("i=2258", "Server time")],
            ..OpcuaClientSpec::default()
        };
        let form = OpcuaClientForm::from_spec("Plant", &saved);
        assert_eq!(form.build().unwrap().1, saved);
    }

    #[test]
    fn endpoints_are_opc_tcp_urls() {
        let mut form = client();
        for bad in ["", "plc.local:4840", "http://plc.local", "opc.tcp://"] {
            form.endpoint = bad.into();
            assert_eq!(form.build().unwrap_err().field, Field::Endpoint, "{bad:?}");
        }
        form.endpoint = " opc.tcp://[fe80::1]:4840/UA ".into();
        assert_eq!(
            form.build().unwrap().1.endpoint,
            "opc.tcp://[fe80::1]:4840/UA"
        );
    }

    #[test]
    fn the_simulator_listens_on_an_ip_address() {
        let mut server = OpcuaServerForm::from_spec("Sim", &OpcuaServerSpec::default());
        assert_eq!(server.bind, "127.0.0.1");
        server.bind = "localhost".into();
        assert_eq!(server.build().unwrap_err().field, Field::Bind);
        server.bind = " [::] ".into();
        assert_eq!(server.build().unwrap().1.bind, "::");
    }

    #[test]
    fn policy_and_mode_are_both_none_or_neither() {
        let mut form = client();
        form.mode = OpcuaSecurityMode::Sign;
        assert_eq!(form.build().unwrap_err().field, Field::Security);
        form.policy = OpcuaSecurityPolicy::Basic256Sha256;
        form.mode = OpcuaSecurityMode::None;
        assert_eq!(form.build().unwrap_err().field, Field::Security);
        form.mode = OpcuaSecurityMode::SignAndEncrypt;
        assert!(form.build().is_ok());
    }

    #[test]
    fn each_sign_in_needs_its_fields() {
        let mut form = client();
        form.auth = OpcuaAuthKind::UserName;
        form.username = "  ".into();
        assert_eq!(form.build().unwrap_err().field, Field::Username);
        form.username = " operator ".into();
        form.password = String::new();
        let (_, spec) = form.build().expect("an empty password is allowed");
        assert_eq!(
            spec.auth,
            OpcuaAuth::UserName {
                username: "operator".into(),
                password: String::new()
            }
        );
        form.auth = OpcuaAuthKind::Certificate;
        assert_eq!(form.build().unwrap_err().field, Field::Certificate);
        form.certificate = "/certs/op.der".into();
        assert_eq!(form.build().unwrap_err().field, Field::PrivateKey);
        form.private_key = "/certs/op.pem".into();
        assert!(matches!(
            form.build().unwrap().1.auth,
            OpcuaAuth::Certificate { .. }
        ));
    }

    #[test]
    fn intervals_have_bounds() {
        let mut form = client();
        form.publishing_ms = "10".into();
        assert_eq!(form.build().unwrap_err().field, Field::PublishingMs);
        form.publishing_ms = "1000".into();
        form.sampling_ms = "0".into();
        assert_eq!(
            form.build().unwrap().1.sampling_ms,
            0,
            "as fast as the server can"
        );
        form.sampling_ms = "-1".into();
        assert_eq!(form.build().unwrap_err().field, Field::SamplingMs);
    }

    #[test]
    fn a_server_user_needs_a_password() {
        let mut server = OpcuaServerForm::from_spec("Sim", &OpcuaServerSpec::default());
        server.password = String::new();
        assert_eq!(server.build().unwrap_err().field, Field::Password);
        server.username = String::new();
        let (_, spec) = server.build().expect("no user at all");
        assert!(spec.username.is_empty() && spec.password.is_empty());
        server.port = "0".into();
        assert_eq!(server.build().unwrap_err().field, Field::Port);
    }
}
