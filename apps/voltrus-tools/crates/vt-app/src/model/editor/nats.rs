//! The NATS client and server simulator fields of the connection editor.

use gpui::{App, AppContext as _, Context, Entity, Window};
use vt_app_core::forms::{NatsAuthKind, NatsClientForm, NatsServerForm, NatsSubjectForm};
use vt_protocol::{NatsClientSpec, NatsServerSpec};
use vt_ui::text_input::{InputKind, TextInput};

use super::{AppModel, Editor, input};

pub struct NatsSubjectRow {
    pub subject: Entity<TextInput>,
    pub queue: Entity<TextInput>,
    pub json_path: Entity<TextInput>,
}

impl NatsSubjectRow {
    fn new(form: NatsSubjectForm, cx: &mut Context<AppModel>) -> Self {
        Self {
            subject: input(
                form.subject,
                "plant.*.power or >",
                InputKind::Text,
                true,
                cx,
            ),
            queue: input(form.queue, "none", InputKind::Text, true, cx),
            json_path: input(form.json_path, "every number", InputKind::Text, true, cx),
        }
    }
}

fn secret(
    text: String,
    placeholder: &'static str,
    cx: &mut Context<AppModel>,
) -> Entity<TextInput> {
    cx.new(|cx| {
        TextInput::new(cx)
            .text(text)
            .placeholder(placeholder)
            .secure()
    })
}

pub struct NatsFields {
    // Client
    pub host: Entity<TextInput>,
    pub port: Entity<TextInput>,
    pub tls: bool,
    pub auth: NatsAuthKind,
    pub token: Entity<TextInput>,
    pub username: Entity<TextInput>,
    pub password: Entity<TextInput>,
    pub seed: Entity<TextInput>,
    /// The chosen `.creds` file's name and contents.
    pub creds_file: String,
    pub creds: String,
    pub client_name: Entity<TextInput>,
    pub subscriptions: Vec<NatsSubjectRow>,
    // Server simulator
    pub bind: Entity<TextInput>,
    pub server_port: Entity<TextInput>,
    pub prefix: Entity<TextInput>,
    pub publish_ms: Entity<TextInput>,
    pub template: Option<String>,
    pub jetstream: bool,
    pub server_username: Entity<TextInput>,
    pub server_password: Entity<TextInput>,
}

impl NatsFields {
    pub(super) fn new(
        client: &NatsClientSpec,
        server: &NatsServerSpec,
        cx: &mut Context<AppModel>,
    ) -> Self {
        let client = NatsClientForm::from_spec("", client);
        let server = NatsServerForm::from_spec("", server);
        Self {
            host: input(client.host, "nats.local", InputKind::Text, true, cx),
            port: input(client.port, "4222", InputKind::Unsigned, true, cx),
            tls: client.tls,
            auth: client.auth,
            token: secret(client.token, "token", cx),
            username: input(client.username, "user", InputKind::Text, false, cx),
            password: secret(client.password, "none", cx),
            seed: secret(client.seed, "SU…", cx),
            creds_file: client.creds_file,
            creds: client.creds,
            client_name: input(client.client_name, "automatic", InputKind::Text, true, cx),
            subscriptions: client
                .subscriptions
                .into_iter()
                .map(|form| NatsSubjectRow::new(form, cx))
                .collect(),
            bind: input(server.bind, "127.0.0.1", InputKind::Text, true, cx),
            server_port: input(server.port, "4222", InputKind::Unsigned, true, cx),
            prefix: input(server.prefix, "plant.inverter", InputKind::Text, true, cx),
            publish_ms: input(server.publish_ms, "1000", InputKind::Unsigned, true, cx),
            template: server.template,
            jetstream: server.jetstream,
            server_username: input(server.username, "anyone", InputKind::Text, false, cx),
            server_password: secret(server.password, "none", cx),
        }
    }

    /// Every text field, for Enter to save and Escape to close.
    pub(super) fn inputs(&self) -> Vec<Entity<TextInput>> {
        let mut fields = vec![
            self.host.clone(),
            self.port.clone(),
            self.token.clone(),
            self.username.clone(),
            self.password.clone(),
            self.seed.clone(),
            self.client_name.clone(),
            self.bind.clone(),
            self.server_port.clone(),
            self.prefix.clone(),
            self.publish_ms.clone(),
            self.server_username.clone(),
            self.server_password.clone(),
        ];
        for row in &self.subscriptions {
            fields.push(row.subject.clone());
            fields.push(row.queue.clone());
            fields.push(row.json_path.clone());
        }
        fields
    }

    fn text(field: &Entity<TextInput>, cx: &App) -> String {
        field.read(cx).value().to_owned()
    }

    pub fn client_form(&self, name: String, cx: &App) -> NatsClientForm {
        NatsClientForm {
            name,
            host: Self::text(&self.host, cx),
            port: Self::text(&self.port, cx),
            tls: self.tls,
            auth: self.auth,
            token: Self::text(&self.token, cx),
            username: Self::text(&self.username, cx),
            password: Self::text(&self.password, cx),
            seed: Self::text(&self.seed, cx),
            creds_file: self.creds_file.clone(),
            creds: self.creds.clone(),
            client_name: Self::text(&self.client_name, cx),
            subscriptions: self
                .subscriptions
                .iter()
                .map(|row| NatsSubjectForm {
                    subject: Self::text(&row.subject, cx),
                    queue: Self::text(&row.queue, cx),
                    json_path: Self::text(&row.json_path, cx),
                })
                .collect(),
        }
    }

    pub fn server_form(&self, name: String, cx: &App) -> NatsServerForm {
        NatsServerForm {
            name,
            bind: Self::text(&self.bind, cx),
            port: Self::text(&self.server_port, cx),
            template: self.template.clone(),
            prefix: Self::text(&self.prefix, cx),
            publish_ms: Self::text(&self.publish_ms, cx),
            jetstream: self.jetstream,
            username: Self::text(&self.server_username, cx),
            password: Self::text(&self.server_password, cx),
        }
    }
}

impl Editor {
    pub fn add_nats_subscription(&mut self, window: &mut Window, cx: &mut Context<AppModel>) {
        let row = NatsSubjectRow::new(NatsSubjectForm::blank(), cx);
        self.watch(&row.subject, window, cx);
        self.watch(&row.queue, window, cx);
        self.watch(&row.json_path, window, cx);
        row.subject.update(cx, |subject, _| subject.focus(window));
        self.nats.subscriptions.push(row);
    }
}
