//! NATS client and server simulator forms.

use vt_protocol::{
    NatsAuth, NatsClientSpec, NatsServerSpec, NatsSubscription, parse_json_path, valid_subject,
    valid_subject_filter,
};

use super::{Field, FormError, error, name, number};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NatsSubjectField {
    Subject,
    Queue,
    JsonPath,
}

/// How the client logs in, as the form's segmented control picks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NatsAuthKind {
    None,
    Token,
    UserPassword,
    Nkey,
    Credentials,
}

impl NatsAuthKind {
    pub const ALL: [NatsAuthKind; 5] = [
        NatsAuthKind::None,
        NatsAuthKind::Token,
        NatsAuthKind::UserPassword,
        NatsAuthKind::Nkey,
        NatsAuthKind::Credentials,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Token => "Token",
            Self::UserPassword => "User",
            Self::Nkey => "NKey",
            Self::Credentials => "Creds file",
        }
    }

    pub fn of(auth: &NatsAuth) -> Self {
        match auth {
            NatsAuth::None => Self::None,
            NatsAuth::Token { .. } => Self::Token,
            NatsAuth::UserPassword { .. } => Self::UserPassword,
            NatsAuth::Nkey { .. } => Self::Nkey,
            NatsAuth::Credentials { .. } => Self::Credentials,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NatsSubjectForm {
    pub subject: String,
    pub queue: String,
    pub json_path: String,
}

impl NatsSubjectForm {
    /// A blank row for a new subscription.
    pub fn blank() -> Self {
        Self {
            subject: String::new(),
            queue: String::new(),
            json_path: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NatsClientForm {
    pub name: String,
    pub host: String,
    pub port: String,
    pub tls: bool,
    pub auth: NatsAuthKind,
    pub token: String,
    pub username: String,
    pub password: String,
    pub seed: String,
    /// The chosen `.creds` file's name and contents.
    pub creds_file: String,
    pub creds: String,
    /// The connection name the server lists.
    pub client_name: String,
    pub subscriptions: Vec<NatsSubjectForm>,
}

impl NatsClientForm {
    pub fn from_spec(name: &str, spec: &NatsClientSpec) -> Self {
        let mut form = Self {
            name: name.to_owned(),
            host: spec.host.clone(),
            port: spec.port.to_string(),
            tls: spec.tls,
            auth: NatsAuthKind::of(&spec.auth),
            token: String::new(),
            username: String::new(),
            password: String::new(),
            seed: String::new(),
            creds_file: String::new(),
            creds: String::new(),
            client_name: spec.name.clone(),
            subscriptions: spec
                .subscriptions
                .iter()
                .map(|subscription| NatsSubjectForm {
                    subject: subscription.subject.clone(),
                    queue: subscription.queue.clone(),
                    json_path: subscription.json_path.clone(),
                })
                .collect(),
        };
        match &spec.auth {
            NatsAuth::None => {}
            NatsAuth::Token { token } => form.token.clone_from(token),
            NatsAuth::UserPassword { username, password } => {
                form.username.clone_from(username);
                form.password.clone_from(password);
            }
            NatsAuth::Nkey { seed } => form.seed.clone_from(seed),
            NatsAuth::Credentials { file, creds } => {
                form.creds_file.clone_from(file);
                form.creds.clone_from(creds);
            }
        }
        form
    }

    pub fn build(&self) -> Result<(String, NatsClientSpec), FormError> {
        let name = name(&self.name)?;
        let host = self.host.trim();
        if host.is_empty() {
            return Err(error(
                Field::Host,
                "Enter the server's IP address or host name",
            ));
        }
        let port = number(&self.port, Field::Port, "Port", 1, u16::MAX)?;
        let auth = match self.auth {
            NatsAuthKind::None => NatsAuth::None,
            NatsAuthKind::Token => {
                if self.token.trim().is_empty() {
                    return Err(error(Field::Credential, "Enter the token"));
                }
                NatsAuth::Token {
                    token: self.token.trim().to_owned(),
                }
            }
            NatsAuthKind::UserPassword => {
                let username = self.username.trim();
                if username.is_empty() {
                    return Err(error(Field::Username, "Enter the user name"));
                }
                NatsAuth::UserPassword {
                    username: username.to_owned(),
                    password: self.password.clone(),
                }
            }
            NatsAuthKind::Nkey => {
                let seed = self.seed.trim();
                if !valid_user_seed(seed) {
                    return Err(error(
                        Field::Credential,
                        "Enter the user's NKey seed: 58 characters starting with SU",
                    ));
                }
                NatsAuth::Nkey {
                    seed: seed.to_owned(),
                }
            }
            NatsAuthKind::Credentials => {
                if !self.creds.contains("NATS USER JWT") || !self.creds.contains("NKEY SEED") {
                    return Err(error(
                        Field::Credential,
                        "Choose a .creds file with a user JWT and NKey seed",
                    ));
                }
                NatsAuth::Credentials {
                    file: self.creds_file.clone(),
                    creds: self.creds.clone(),
                }
            }
        };
        let mut subscriptions = Vec::with_capacity(self.subscriptions.len());
        for (index, row) in self.subscriptions.iter().enumerate() {
            let subject = row.subject.trim();
            if !valid_subject_filter(subject) {
                return Err(error(
                    Field::NatsSubject(index, NatsSubjectField::Subject),
                    if subject.is_empty() {
                        "Enter a subject, e.g. plant.*.power or >".to_owned()
                    } else {
                        format!(
                            "“{subject}” is not a subject: tokens are split by dots, * takes a whole token and > goes last"
                        )
                    },
                ));
            }
            let queue = row.queue.trim();
            if queue.contains(char::is_whitespace) {
                return Err(error(
                    Field::NatsSubject(index, NatsSubjectField::Queue),
                    "A queue group name has no spaces",
                ));
            }
            let json_path = row.json_path.trim();
            if let Err(problem) = parse_json_path(json_path) {
                return Err(error(
                    Field::NatsSubject(index, NatsSubjectField::JsonPath),
                    format!("JSON path {problem}"),
                ));
            }
            subscriptions.push(NatsSubscription {
                subject: subject.to_owned(),
                queue: queue.to_owned(),
                json_path: json_path.to_owned(),
            });
        }
        Ok((
            name,
            NatsClientSpec {
                host: host.to_owned(),
                port,
                tls: self.tls,
                auth,
                name: self.client_name.trim().to_owned(),
                subscriptions,
            },
        ))
    }
}

/// An NKey user seed: base32, 58 characters, `S` (seed) then `U` (user).
fn valid_user_seed(seed: &str) -> bool {
    seed.len() == 58
        && seed.starts_with("SU")
        && seed
            .chars()
            .all(|c| c.is_ascii_uppercase() || ('2'..='7').contains(&c))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NatsServerForm {
    pub name: String,
    pub bind: String,
    pub port: String,
    pub template: Option<String>,
    pub prefix: String,
    pub publish_ms: String,
    pub jetstream: bool,
    pub username: String,
    pub password: String,
}

impl NatsServerForm {
    pub fn from_spec(name: &str, spec: &NatsServerSpec) -> Self {
        Self {
            name: name.to_owned(),
            bind: spec.bind.clone(),
            port: spec.port.to_string(),
            template: spec.template.clone(),
            prefix: spec.prefix.clone(),
            publish_ms: spec.publish_ms.to_string(),
            jetstream: spec.jetstream,
            username: spec.username.clone(),
            password: spec.password.clone(),
        }
    }

    pub fn build(&self) -> Result<(String, NatsServerSpec), FormError> {
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
        let prefix = self.prefix.trim().trim_matches('.');
        if self.template.is_some() && !valid_subject(prefix) {
            return Err(error(
                Field::Prefix,
                "Enter the subject the device publishes under, e.g. plant.inverter (no * or >)",
            ));
        }
        let publish_ms = number(
            &self.publish_ms,
            Field::PublishMs,
            "Publish interval",
            NatsServerSpec::MIN_PUBLISH_MS,
            60_000,
        )?;
        let username = self.username.trim();
        if username.is_empty() && !self.password.is_empty() {
            return Err(error(Field::Username, "A password needs a username"));
        }
        if username.contains(char::is_whitespace) {
            return Err(error(Field::Username, "A username has no spaces"));
        }
        Ok((
            name,
            NatsServerSpec {
                bind: ip.to_string(),
                port,
                template: self.template.clone(),
                prefix: prefix.to_owned(),
                publish_ms,
                jetstream: self.jetstream && self.template.is_some(),
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

    const SEED: &str = "SUAMLK2ZNL35WSMW37E7UD4VZ7ELPKW7DHC3BWBSD2GCZ7IUQQXZIORRBU";

    #[test]
    fn default_forms_round_trip_to_the_same_specs() {
        let client = NatsClientSpec::default();
        assert_eq!(
            NatsClientForm::from_spec("Server", &client).build(),
            Ok(("Server".to_owned(), client))
        );
        let server = NatsServerSpec::default();
        assert_eq!(
            NatsServerForm::from_spec("Sim", &server).build(),
            Ok(("Sim".to_owned(), server))
        );
    }

    #[test]
    fn every_auth_kind_round_trips() {
        for auth in [
            NatsAuth::Token {
                token: "t0k3n".into(),
            },
            NatsAuth::UserPassword {
                username: "plant".into(),
                password: " spaced ".into(),
            },
            NatsAuth::Nkey { seed: SEED.into() },
            NatsAuth::Credentials {
                file: "plant.creds".into(),
                creds: "-----BEGIN NATS USER JWT-----\nx\n------END NATS USER JWT------\n\
                        -----BEGIN USER NKEY SEED-----\ny\n------END USER NKEY SEED------"
                    .into(),
            },
        ] {
            let spec = NatsClientSpec {
                auth,
                ..NatsClientSpec::default()
            };
            assert_eq!(
                NatsClientForm::from_spec("S", &spec)
                    .build()
                    .map(|(_, spec)| spec),
                Ok(spec)
            );
        }
    }

    #[test]
    fn client_errors_point_at_the_offending_field() {
        let form = || NatsClientForm::from_spec("Server", &NatsClientSpec::default());

        let mut bad = form();
        bad.subscriptions[0].subject = "plant.>.power".into();
        assert_eq!(
            bad.build().unwrap_err().field,
            Field::NatsSubject(0, NatsSubjectField::Subject)
        );

        let mut bad = form();
        bad.subscriptions.push(NatsSubjectForm {
            subject: "meter".into(),
            queue: "two words".into(),
            json_path: String::new(),
        });
        assert_eq!(
            bad.build().unwrap_err().field,
            Field::NatsSubject(1, NatsSubjectField::Queue)
        );

        let mut bad = form();
        bad.subscriptions[0].json_path = "phases[x]".into();
        assert_eq!(
            bad.build().unwrap_err().field,
            Field::NatsSubject(0, NatsSubjectField::JsonPath)
        );

        let mut bad = form();
        bad.auth = NatsAuthKind::Token;
        assert_eq!(bad.build().unwrap_err().field, Field::Credential);

        let mut bad = form();
        bad.auth = NatsAuthKind::UserPassword;
        bad.password = "secret".into();
        assert_eq!(bad.build().unwrap_err().field, Field::Username);

        let mut bad = form();
        bad.auth = NatsAuthKind::Nkey;
        bad.seed = SEED.replacen("SU", "SA", 1);
        assert_eq!(
            bad.build().unwrap_err().field,
            Field::Credential,
            "an account seed is not a user seed"
        );

        let mut bad = form();
        bad.auth = NatsAuthKind::Credentials;
        bad.creds = "not a creds file".into();
        assert_eq!(bad.build().unwrap_err().field, Field::Credential);

        let mut bad = form();
        bad.host = "  ".into();
        assert_eq!(bad.build().unwrap_err().field, Field::Host);
    }

    #[test]
    fn a_client_may_only_publish_and_request() {
        let mut form = NatsClientForm::from_spec("Server", &NatsClientSpec::default());
        form.subscriptions.clear();
        form.subscriptions.push(NatsSubjectForm {
            subject: " plant.*.power ".into(),
            queue: " workers ".into(),
            json_path: " ac.power ".into(),
        });
        let (_, spec) = form.build().expect("a valid form");
        assert_eq!(
            spec.subscriptions,
            vec![NatsSubscription {
                subject: "plant.*.power".into(),
                queue: "workers".into(),
                json_path: "ac.power".into(),
            }]
        );
        form.subscriptions.clear();
        assert!(
            form.build()
                .expect("no subscriptions")
                .1
                .subscriptions
                .is_empty()
        );
    }

    #[test]
    fn server_prefixes_are_concrete_subjects() {
        let mut form = NatsServerForm::from_spec("Sim", &NatsServerSpec::default());
        form.prefix = ".plant.inverter.".into();
        assert_eq!(
            form.build().expect("a valid prefix").1.prefix,
            "plant.inverter"
        );
        form.prefix = "plant.*".into();
        assert_eq!(form.build().unwrap_err().field, Field::Prefix);
        // A server without a device publishes nothing: no prefix needed and
        // no stream to keep.
        form.template = None;
        let (_, spec) = form.build().expect("a plain server");
        assert!(!spec.jetstream);
        form.password = "pw".into();
        assert_eq!(form.build().unwrap_err().field, Field::Username);
        form.username = "plant".into();
        assert_eq!(form.build().expect("with a user").1.username, "plant");
        form.bind = "localhost".into();
        assert_eq!(form.build().unwrap_err().field, Field::Bind);
        form.bind = "[::1]".into();
        form.publish_ms = "50".into();
        assert_eq!(form.build().unwrap_err().field, Field::PublishMs);
    }
}
