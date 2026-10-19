//! Interactive terminal management over the same private API as the desktop.
mod api;
use api::Api;
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
    },
    execute,
};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    io::{self, IsTerminal},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const PAGES: [&str; 8] = [
    "Overview",
    "Pair with QR",
    "Pending approvals",
    "Machines & devices",
    "Local services",
    "Pair remote host",
    "Owner operations",
    "Diagnostics",
];
#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Home,
    Qr,
    Pending,
    Machines,
    Services,
    Pair,
    Owners,
    Diagnostics,
}
impl Page {
    fn index(self) -> usize {
        self as usize
    }
    fn at(n: usize) -> Self {
        [
            Self::Home,
            Self::Qr,
            Self::Pending,
            Self::Machines,
            Self::Services,
            Self::Pair,
            Self::Owners,
            Self::Diagnostics,
        ][n % 8]
    }
}
#[derive(Clone)]
enum Task {
    Status,
    Request(Value),
    Online(bool),
    Remove {
        peer: String,
        revision: u64,
        client: bool,
    },
    Pair {
        preview: Value,
        document: String,
    },
    Open {
        host: String,
        service: Value,
        name: String,
        enable: bool,
    },
    OwnerList,
    OwnerStart(Value),
    OwnerRetry(String),
}
#[derive(Clone)]
enum Kind {
    Status,
    Saved,
    Qr,
    Catalog(String),
    Preview(String),
    Paired,
    Open,
    Owners,
    OwnerSaved,
}
#[derive(Clone)]
struct Work {
    kind: Kind,
    task: Task,
}
struct Reply {
    kind: Kind,
    result: Result<Value, String>,
}
fn perform(api: &Api, task: Task) -> Result<Value, String> {
    match task {
        Task::Status => api.request(json!({"op":"status"})),
        Task::Request(v) => api.request(v),
        Task::Online(v) => api.online(v),
        Task::Remove {
            peer,
            revision,
            client,
        } => {
            if client {
                api.request(json!({"op":"mutate","operation":{"op":"revoke","peer":peer,"expected_revision":revision}}))?;
            }
            api.request(json!({"op":"unpair","host":peer}))
                .map_err(|e| {
                    if client {
                        format!(
                            "Local grants revoked; forgetting the saved host was not confirmed: {e}"
                        )
                    } else {
                        e
                    }
                })
        }
        Task::Pair { preview, document } => {
            let host = text(&preview, "host");
            let value = api.request(json!({"op":"pair","host":host,"invitation":document}))?;
            api.save_pair(host, text(&preview, "host_name"))?;
            Ok(value)
        }
        Task::Open {
            host,
            service,
            name,
            enable,
        } => {
            if service["kind"] != "http" {
                return Err(
                    "TCP services use wispnet tunnel; browser access supports HTTP services".into(),
                );
            }
            if enable {
                api.request(json!({"op":"enable"}))?;
            }
            let result=api.request(json!({"op":"open","host":host,"service":service["id"],"incarnation":service["incarnation"],"host_name":name}))?;
            // Stable local URLs contain no capability; never invoke a shell.
            let url = text(&result, "url");
            let parsed = url::Url::parse(url).map_err(|_| "Invalid browser handoff")?;
            if parsed.scheme() != "http"
                || !parsed.host_str().is_some_and(|h| h.ends_with(".localhost"))
                || parsed.port().is_none()
                || parsed.path() != "/"
                || parsed.fragment().is_some()
                || parsed.query().is_some()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
            {
                return Err("Invalid local browser handoff".into());
            }
            if std::env::var_os("DISPLAY").is_some()
                || std::env::var_os("WAYLAND_DISPLAY").is_some()
            {
                api::command(std::process::Command::new("xdg-open").arg(url), None, 10)?;
            }
            Ok(result)
        }
        Task::OwnerList => api.owner(&["--operations".into()]),
        Task::OwnerStart(plan) => api.owner_start(&plan),
        Task::OwnerRetry(id) => api.owner(&["--retry".into(), id]),
    }
}
fn worker(api: Api) -> (mpsc::Sender<Work>, mpsc::Receiver<Reply>) {
    let (tx, jobs) = mpsc::channel::<Work>();
    let (done, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for work in jobs {
            let result = perform(&api, work.task);
            if done
                .send(Reply {
                    kind: work.kind,
                    result,
                })
                .is_err()
            {
                break;
            }
        }
    });
    (tx, rx)
}
#[derive(Clone)]
enum Action {
    Nav(Page),
    Qr,
    Pending(Value),
    Machine(String),
    Service(Value),
    Remote(String, Value),
    PairInput(bool),
    Form(Form),
    Confirm {
        title: String,
        detail: Vec<String>,
        work: Work,
    },
    Run(Work),
    OwnerNew(bool),
    OwnerDetail(Value),
    Info(String, Vec<String>),
}
#[derive(Clone)]
struct Entry {
    id: String,
    label: String,
    detail: Vec<String>,
    action: Action,
}
#[derive(Clone)]
struct Field {
    label: String,
    value: String,
    kind: FieldKind,
}
#[derive(Clone)]
enum FieldKind {
    Text(usize),
    Check(bool),
    Choice(Vec<String>, usize),
}
impl Field {
    fn text(label: &str, value: &str, limit: usize) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            kind: FieldKind::Text(limit),
        }
    }
    fn check(label: &str, value: Value, checked: bool) -> Self {
        Self {
            label: label.into(),
            value: value.to_string(),
            kind: FieldKind::Check(checked),
        }
    }
    fn choice(label: &str, choices: &[&str], index: usize) -> Self {
        Self {
            label: label.into(),
            value: String::new(),
            kind: FieldKind::Choice(choices.iter().map(|s| s.to_string()).collect(), index),
        }
    }
}
#[derive(Clone)]
enum FormKind {
    Service {
        existing: Option<String>,
        revision: u64,
    },
    Pair {
        file: bool,
    },
    Owner {
        approve: bool,
    },
}
#[derive(Clone)]
struct Form {
    title: String,
    kind: FormKind,
    fields: Vec<Field>,
    selected: usize,
}
enum Modal {
    Choice {
        title: String,
        detail: Vec<String>,
        entries: Vec<Entry>,
        selected: usize,
        scroll: u16,
    },
    Form(Form),
    Info {
        title: String,
        lines: Vec<String>,
        scroll: u16,
    },
}
#[derive(Clone)]
enum Hit {
    Nav(usize),
    Row(usize),
    ModalRow(usize),
    Field(usize),
}
struct App {
    api: Api,
    page: Page,
    menu_focus: bool,
    menu: usize,
    selected: usize,
    list_state: ListState,
    snapshot: Option<Value>,
    online: bool,
    names: Value,
    catalogs: HashMap<String, Value>,
    catalog_errors: HashMap<String, String>,
    qr: Option<Value>,
    operations: Vec<Value>,
    modal: Option<Modal>,
    message: String,
    tx: mpsc::Sender<Work>,
    rx: mpsc::Receiver<Reply>,
    busy: bool,
    pending_work: Option<Work>,
    last_poll: Instant,
    hits: Vec<(Rect, Hit)>,
    last_click: Option<(Instant, usize)>,
}
fn text<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}
fn array(v: &Value) -> Vec<Value> {
    v.as_array().cloned().unwrap_or_default()
}
fn clean(s: &str) -> String {
    s.chars()
        .filter(|c| {
            !c.is_control() && !matches!(*c,'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}')
        })
        .collect()
}
fn short(s: &str) -> String {
    s.chars().take(12).collect()
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn entry(id: &str, label: impl Into<String>, action: Action) -> Entry {
    Entry {
        id: id.into(),
        label: label.into(),
        detail: vec![],
        action,
    }
}
impl App {
    fn new(api: Api) -> Self {
        let names = api.names();
        let (tx, rx) = worker(api.clone());
        Self {
            api,
            page: Page::Home,
            menu_focus: true,
            menu: 0,
            selected: 0,
            list_state: ListState::default(),
            snapshot: None,
            online: false,
            names,
            catalogs: HashMap::new(),
            catalog_errors: HashMap::new(),
            qr: None,
            operations: vec![],
            modal: None,
            message: "Connecting to local node…".into(),
            tx,
            rx,
            busy: false,
            pending_work: None,
            last_poll: Instant::now() - Duration::from_secs(5),
            hits: vec![],
            last_click: None,
        }
    }
    fn status(&self) -> Value {
        self.snapshot.clone().unwrap_or_else(|| json!({}))
    }
    fn host(&self) -> Value {
        self.status()["host"].clone()
    }
    fn revision(&self) -> u64 {
        self.host()["revision"].as_u64().unwrap_or(0)
    }
    fn clients(&self) -> Vec<Value> {
        array(&self.host()["clients"])
    }
    fn services(&self) -> Vec<Value> {
        array(&self.host()["services"])
    }
    fn pending(&self) -> Vec<Value> {
        array(&self.status()["invitations"])
            .into_iter()
            .filter(|v| v["state"] == "pending")
            .collect()
    }
    fn name(&self, peer: &str) -> String {
        self.names[peer]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(clean)
            .or_else(|| {
                self.clients()
                    .iter()
                    .find(|v| v["peer"] == peer)
                    .map(|v| clean(text(v, "name")))
            })
            .unwrap_or_else(|| format!("Host {}", short(peer)))
    }
    fn mutate(&self, operation: Value) -> Work {
        Work {
            kind: Kind::Saved,
            task: Task::Request(json!({"op":"mutate","operation":operation})),
        }
    }
    fn send(&mut self, work: Work) {
        if self.busy {
            if !matches!(work.kind, Kind::Status) && self.pending_work.is_none() {
                self.pending_work = Some(work);
                self.message = "Queued after the current operation…".into();
            } else {
                self.message = "An operation is already queued; please wait".into();
            }
            return;
        }
        let allow_offline = matches!(
            work.task,
            Task::Status
                | Task::Online(_)
                | Task::OwnerList
                | Task::OwnerStart(_)
                | Task::OwnerRetry(_)
        );
        if !self.online && !allow_offline {
            self.message = "Node offline. Go online or refresh before making changes.".into();
            return;
        }
        let is_status = matches!(work.kind, Kind::Status);
        if self.tx.send(work).is_ok() {
            self.busy = true;
            if !is_status {
                self.message = "Working… (UI remains responsive)".into();
            }
        }
    }
    fn navigate(&mut self, page: Page) {
        if page != Page::Qr {
            self.qr = None;
        }
        self.page = page;
        self.menu = page.index();
        self.selected = 0;
        self.list_state = ListState::default();
        self.menu_focus = false;
        if page == Page::Qr {
            self.action(Action::Qr);
        }
        if page == Page::Owners {
            self.send(Work {
                kind: Kind::Owners,
                task: Task::OwnerList,
            });
        }
    }
    fn entries(&self) -> Vec<Entry> {
        match self.page {
            Page::Home => vec![
                entry("qr", "Generate pairing QR", Action::Qr),
                entry(
                    "pending",
                    format!("Pending approvals ({})", self.pending().len()),
                    Action::Nav(Page::Pending),
                ),
                entry(
                    "machines",
                    "Manage machines and devices",
                    Action::Nav(Page::Machines),
                ),
                entry(
                    "services",
                    "Manage local services and grants",
                    Action::Nav(Page::Services),
                ),
                entry("pair", "Pair a remote host", Action::Nav(Page::Pair)),
                entry(
                    "online",
                    if self.online {
                        "Go offline"
                    } else {
                        "Go online"
                    },
                    Action::Confirm {
                        title: if self.online {
                            "Go offline"
                        } else {
                            "Go online"
                        }
                        .into(),
                        detail: vec![
                            "Start/stop this computer's node. The relay remains independent."
                                .into(),
                        ],
                        work: Work {
                            kind: Kind::Saved,
                            task: Task::Online(!self.online),
                        },
                    },
                ),
                entry(
                    "access",
                    if self.status()["access_enabled"] == true {
                        "Stop browser access"
                    } else {
                        "Enable browser access"
                    },
                    Action::Run(Work {
                        kind: Kind::Saved,
                        task: Task::Request(
                            json!({"op":if self.status()["access_enabled"]==true{"stop"}else{"enable"}}),
                        ),
                    }),
                ),
                entry(
                    "owners",
                    "Combined owner operations",
                    Action::Nav(Page::Owners),
                ),
            ],
            Page::Pending => self
                .pending()
                .into_iter()
                .map(|v| {
                    let mut row = entry(
                        text(&v, "invitation"),
                        format!("{} · {}", clean(text(&v, "name")), short(text(&v, "peer"))),
                        Action::Pending(v.clone()),
                    );
                    row.detail = vec![
                        format!("Client ID: {}", text(&v, "peer")),
                        format!(
                            "Expires in {} seconds",
                            v["expires_at"].as_u64().unwrap_or(0).saturating_sub(now())
                        ),
                        "Approve pairing, then explicitly select service grants.".into(),
                    ];
                    row
                })
                .collect(),
            Page::Machines => {
                let mut peers = BTreeMap::new();
                for pin in array(&self.status()["peer_pins"]) {
                    if let Some(id) = pin.as_str() {
                        peers.insert(id.to_string(), false);
                    }
                }
                for client in self.clients() {
                    peers.insert(text(&client, "peer").to_owned(), true);
                }
                peers.into_iter().map(|(peer,client)|{
                    let status=self.status();let telemetry=&status["peer_telemetry"][&peer];
                    let activity=if !self.online{"Node offline".to_string()}else if status["connected_peers"][&peer].as_u64().unwrap_or(0)>0{"Connected".into()}else if let Some(age)=status["recent_peers"][&peer].as_u64(){format!("Seen {age}s ago")}else{"Idle".into()};
                    let path=if telemetry.is_object(){format!(" · {} · {}ms",text(telemetry,"path").to_uppercase(),telemetry["rtt_ms"])}else{String::new()};
                    let mut row=entry(&peer,format!("{} · {} · {activity}{path}",self.name(&peer),short(&peer)),Action::Machine(peer.clone()));
                    row.detail=vec![format!("Machine ID: {peer}"),format!("Approved client here: {}",if client{"yes"}else{"no"}),"Remove revokes this node's grants and forgets its pin/routes; remote permissions are managed there.".into()];
                    if let Some(error)=self.catalog_errors.get(&peer){row.detail.push(format!("Catalog unavailable: {}",clean(error)));}row
                }).collect()
            }
            Page::Services => {
                let mut rows = vec![entry(
                    "+",
                    "+ Add local service",
                    Action::Form(self.service_form(None)),
                )];
                rows.extend(self.services().into_iter().map(|service| {
                    let mut row = entry(
                        text(&service, "id"),
                        format!(
                            "{} · {} · {} · {} permitted devices",
                            clean(text(&service, "display_name")),
                            text(&service, "backend"),
                            text(&service, "kind"),
                            array(&service["allow"]).len()
                        ),
                        Action::Service(service.clone()),
                    );
                    row.detail = vec![
                        format!("Service ID: {}", text(&service, "id")),
                        format!("Backend: {}", text(&service, "backend")),
                        format!(
                            "Permitted devices: {}",
                            array(&service["allow"])
                                .iter()
                                .filter_map(Value::as_str)
                                .map(|p| self.name(p))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    ];
                    row
                }));
                rows
            }
            Page::Pair => vec![
                entry(
                    "paste",
                    "Paste a pairing code / invitation JSON",
                    Action::PairInput(false),
                ),
                entry(
                    "file",
                    "Import a private invitation file",
                    Action::PairInput(true),
                ),
            ],
            Page::Owners => {
                let mut rows = vec![
                    entry(
                        "approve",
                        "Add a client across selected owner hosts",
                        Action::OwnerNew(true),
                    ),
                    entry(
                        "revoke",
                        "Revoke a client across selected owner hosts",
                        Action::OwnerNew(false),
                    ),
                ];
                rows.extend(self.operations.iter().map(|v| {
                    entry(
                        text(v, "id"),
                        format!(
                            "{} · {} · {}",
                            short(text(v, "id")),
                            text(&v["action"], "op"),
                            if v["complete"] == true {
                                "All hosts acknowledged"
                            } else {
                                "Pending or rejected hosts"
                            }
                        ),
                        Action::OwnerDetail(v.clone()),
                    )
                }));
                rows
            }
            Page::Diagnostics => {
                let s = self.status();
                vec![entry(
                    "info",
                    "Show node details",
                    Action::Info(
                        "Node diagnostics".into(),
                        vec![
                            format!("Host: {}", text(&s["host"], "host")),
                            format!("Name: {}", text(&s, "host_name")),
                            format!("Revision: {}", s["host"]["revision"]),
                            format!("Relay: {}", text(&s, "relay")),
                            format!("Transport streams: {}", s["transport_streams"]),
                            format!("Config: {}", self.api.config.display()),
                            format!("State: {}", self.api.state.display()),
                            "A recent request or a connected transport never grants authorization."
                                .into(),
                        ],
                    ),
                )]
            }
            Page::Qr => vec![],
        }
    }
    fn service_form(&self, existing: Option<Value>) -> Form {
        let old = existing.clone().unwrap_or_else(|| json!({}));
        let mut fields = vec![
            Field::text("Service name", text(&old, "display_name"), 128),
            Field::text("Local port / loopback address", text(&old, "backend"), 128),
            Field::choice(
                "Protocol",
                &["http", "tcp"],
                usize::from(old["kind"] == "tcp"),
            ),
        ];
        for client in self.clients() {
            let checked = existing.is_none() || array(&old["allow"]).contains(&client["peer"]);
            fields.push(Field::check(
                &format!(
                    "Allow {} · {}",
                    clean(text(&client, "name")),
                    short(text(&client, "peer"))
                ),
                client["peer"].clone(),
                checked,
            ));
        }
        Form {
            title: if existing.is_some() {
                "Edit service and device grants"
            } else {
                "Add service"
            }
            .into(),
            kind: FormKind::Service {
                existing: existing.map(|v| text(&v, "id").to_owned()),
                revision: self.revision(),
            },
            fields,
            selected: 0,
        }
    }
    fn choice(&mut self, title: impl Into<String>, detail: Vec<String>, entries: Vec<Entry>) {
        self.modal = Some(Modal::Choice {
            title: title.into(),
            detail,
            entries,
            selected: 0,
            scroll: 0,
        });
    }
    fn confirmation(&mut self, title: String, detail: Vec<String>, work: Work) {
        self.choice(
            title,
            detail,
            vec![
                entry("confirm", "Confirm", Action::Run(work)),
                entry(
                    "cancel",
                    "Cancel",
                    Action::Info("Cancelled".into(), vec!["No changes made.".into()]),
                ),
            ],
        );
    }
    fn action(&mut self, action: Action) {
        match action {
            Action::Nav(page) => self.navigate(page),
            Action::Qr => {
                self.page = Page::Qr;
                self.menu = 1;
                self.send(Work {
                    kind: Kind::Qr,
                    task: Task::Request(json!({"op":"invite_qr"})),
                });
            }
            Action::Run(work) => {
                self.modal = None;
                self.send(work);
            }
            Action::Confirm {
                title,
                detail,
                work,
            } => self.confirmation(title, detail, work),
            Action::Info(title, lines) => {
                self.modal = Some(Modal::Info {
                    title,
                    lines,
                    scroll: 0,
                })
            }
            Action::Form(form) => self.modal = Some(Modal::Form(form)),
            Action::PairInput(file) => {
                self.modal = Some(Modal::Form(Form {
                    title: if file {
                        "Import invitation"
                    } else {
                        "Paste invitation"
                    }
                    .into(),
                    kind: FormKind::Pair { file },
                    fields: vec![Field::text(
                        if file {
                            "Absolute private file path"
                        } else {
                            "Pairing code or invitation JSON"
                        },
                        "",
                        8192,
                    )],
                    selected: 0,
                }))
            }
            Action::Pending(v) => {
                let detail=vec![format!("Device: {}",clean(text(&v,"name"))),format!("Full client ID: {}",text(&v,"peer")),format!("Invitation: {}",text(&v,"invitation")),"Approval grants pairing only. Configure each service's permitted devices afterward.".into()];
                self.choice("Review pairing request",detail.clone(),vec![entry("approve","Approve this device",Action::Confirm{title:"Approve pairing".into(),detail:detail.clone(),work:self.mutate(json!({"op":"approve_invitation","invitation":v["invitation"],"expected_revision":self.revision()}))}),entry("reject","Reject this request",Action::Confirm{title:"Reject pairing".into(),detail,work:self.mutate(json!({"op":"reject_invitation","invitation":v["invitation"],"expected_revision":self.revision()}))})]);
            }
            Action::Machine(peer) => {
                let client = self.clients().iter().any(|v| v["peer"] == peer);
                let pinned = array(&self.status()["peer_pins"]).contains(&json!(peer));
                let detail = vec![
                    self.name(&peer),
                    format!("Full machine ID: {peer}"),
                    format!("Approved client here: {client}; saved host pin: {pinned}"),
                ];
                let mut choices = vec![];
                if pinned {
                    choices.push(entry(
                        "catalog",
                        "Refresh and browse remote services",
                        Action::Run(Work {
                            kind: Kind::Catalog(peer.clone()),
                            task: Task::Request(json!({"op":"catalog","host":peer})),
                        }),
                    ));
                }
                choices.push(entry("remove","Remove machine / revoke local access",Action::Confirm{title:"Remove this machine".into(),detail:vec![self.name(&peer),format!("Full ID: {peer}"),"Revokes its service grants here, ends active access, and forgets its saved pin and browser routes. Permissions on the remote host remain managed there.".into()],work:Work{kind:Kind::Saved,task:Task::Remove{peer,client,revision:self.revision()}}}));
                self.choice("Manage machine", detail, choices);
            }
            Action::Service(service) => {
                let details = vec![
                    format!("Service: {}", clean(text(&service, "display_name"))),
                    format!("ID: {}", text(&service, "id")),
                    format!("Backend: {}", text(&service, "backend")),
                ];
                let mut choices = vec![entry(
                    "edit",
                    "Edit service and permitted devices",
                    Action::Form(self.service_form(Some(service.clone()))),
                )];
                if service["kind"] == "http" {
                    choices.push(entry(
                        "open",
                        "Open service / show browser URL",
                        Action::Remote(text(&self.host(), "host").into(), service.clone()),
                    ));
                }
                choices.push(entry("remove","Remove service",Action::Confirm{title:"Remove local service".into(),detail:details.clone(),work:self.mutate(json!({"op":"remove","id":service["id"],"expected_revision":self.revision()}))}));
                self.choice("Manage service", details, choices);
            }
            Action::Remote(host, service) => {
                self.send(Work {
                    kind: Kind::Open,
                    task: Task::Open {
                        host: host.clone(),
                        service,
                        name: if host == text(&self.host(), "host") {
                            text(&self.status(), "host_name").into()
                        } else {
                            self.name(&host)
                        },
                        enable: self.status()["access_enabled"] != true,
                    },
                });
            }
            Action::OwnerNew(approve) => {
                let path = self
                    .api
                    .config
                    .parent()
                    .unwrap_or(&self.api.config)
                    .join("admin-targets.json");
                let targets = if path.exists() {
                    match api::read_private(&path, 65536).and_then(|s| {
                        serde_json::from_str::<Value>(&s)
                            .map_err(|_| "Invalid owner targets file".into())
                    }) {
                        Ok(v) if v.is_array() && !array(&v).is_empty() => array(&v),
                        Ok(_) => {
                            self.message = "Owner targets must be a nonempty array".into();
                            return;
                        }
                        Err(error) => {
                            self.message = error;
                            return;
                        }
                    }
                } else {
                    vec![json!({"host":self.host()["host"]})]
                };
                let mut seen = std::collections::HashSet::new();
                let valid = targets.len() <= 32
                    && targets.iter().all(|target| {
                        target.as_object().is_some_and(|object| {
                            object.keys().all(|k| matches!(k.as_str(), "host" | "ssh"))
                        }) && text(target, "host")
                            .parse::<wispnet_core::identity::EndpointId>()
                            .is_ok()
                            && seen.insert(text(target, "host").to_owned())
                            && target.get("ssh").is_none_or(|v| {
                                v.as_str().is_some_and(|s| {
                                    !s.is_empty()
                                        && s.len() <= 254
                                        && s.as_bytes()[0].is_ascii_alphanumeric()
                                        && s.bytes().all(|c| {
                                            c.is_ascii_alphanumeric() || b"_.@-".contains(&c)
                                        })
                                })
                            })
                    });
                if !valid {
                    self.message = "Invalid owner targets. Use unique complete host IDs and explicit trusted SSH routes; go online to discover this host's ID.".into();
                    return;
                }
                let mut fields = vec![Field::text("Client public ID", "", 64)];
                if approve {
                    fields.push(Field::text("Client name", "", 128));
                }
                for target in targets {
                    fields.push(Field::check(
                        &format!(
                            "Target {} · {}",
                            if text(&target, "ssh").is_empty() {
                                "This host"
                            } else {
                                text(&target, "ssh")
                            },
                            short(text(&target, "host"))
                        ),
                        target,
                        false,
                    ));
                }
                self.modal = Some(Modal::Form(Form {
                    title: if approve {
                        "Add client across hosts"
                    } else {
                        "Revoke client across hosts"
                    }
                    .into(),
                    kind: FormKind::Owner { approve },
                    fields,
                    selected: 0,
                }));
            }
            Action::OwnerDetail(value) => {
                let mut detail = vec![
                    format!("Operation ID: {}", text(&value, "id")),
                    format!("Action: {}", value["action"]),
                ];
                for target in array(&value["targets"]) {
                    detail.push(format!(
                        "{} · {} · revision {} · {}",
                        text(&target, "host"),
                        text(&target, "state"),
                        target["revision"],
                        clean(text(&target, "reason"))
                    ));
                }
                self.choice(
                    "Owner acknowledgements",
                    detail,
                    vec![entry(
                        "retry",
                        "Retry only pending owner targets",
                        Action::Run(Work {
                            kind: Kind::OwnerSaved,
                            task: Task::OwnerRetry(text(&value, "id").into()),
                        }),
                    )],
                );
            }
        }
    }
    fn save_form(&mut self, form: Form) -> Result<(), String> {
        match form.kind {
            FormKind::Service { existing, revision } => {
                let name = form.fields[0].value.trim();
                if !wispnet_core::enrollment::valid_name(name) {
                    return Err("Enter a valid service name".into());
                }
                let port = form.fields[1].value.trim();
                let backend = if port.chars().all(|c| c.is_ascii_digit()) {
                    format!("127.0.0.1:{port}")
                } else if port.starts_with(':') {
                    format!("127.0.0.1{port}")
                } else {
                    port.into()
                };
                let address: std::net::SocketAddr = backend
                    .parse()
                    .map_err(|_| "Enter a loopback address or local port (for example 8201)")?;
                if !address.ip().is_loopback() || address.port() == 0 {
                    return Err(
                        "Service backend must be a loopback address with a nonzero port".into(),
                    );
                }
                let FieldKind::Choice(ref choices, index) = form.fields[2].kind else {
                    return Err("Missing protocol".into());
                };
                let mut id = String::new();
                for c in name.to_lowercase().chars() {
                    if c.is_ascii_alphanumeric() {
                        id.push(c);
                    } else if !id.ends_with('-') {
                        id.push('-');
                    }
                }
                let id = existing
                    .clone()
                    .unwrap_or_else(|| id.trim_matches('-').chars().take(64).collect());
                if !wispnet_core::protocol::valid_service_id(&id) {
                    return Err(
                        "Service name must produce a valid ID; use some ASCII letters or numbers"
                            .into(),
                    );
                }
                let allow: Vec<Value> = form
                    .fields
                    .iter()
                    .skip(3)
                    .filter(|f| matches!(f.kind, FieldKind::Check(true)))
                    .map(|f| {
                        serde_json::from_str(&f.value)
                            .map_err(|_| "Invalid selected client".to_string())
                    })
                    .collect::<Result<_, _>>()?;
                let replace = existing.is_none() && self.services().iter().any(|s| s["id"] == id);
                let detail = vec![
                    format!("Service: {name} · {id}"),
                    format!("Backend: {backend} · {}", choices[index]),
                    format!(
                        "Permitted devices: {}",
                        allow
                            .iter()
                            .filter_map(Value::as_str)
                            .map(|p| format!("{} ({p})", self.name(p)))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    if replace {
                        "An existing service has this ID. Confirming replaces its backend and entire access list.".into()
                    } else {
                        "Only the selected devices will be allowed.".into()
                    },
                ];
                self.confirmation(if replace{"Replace existing service"}else{"Save service and grants"}.into(),detail,self.mutate(json!({"op":"service","id":id,"backend":backend,"display_name":name,"kind":choices[index],"allow":allow,"expected_revision":revision})));
            }
            FormKind::Pair { file } => {
                let value = form.fields[0].value.trim();
                let document = if file {
                    let path = PathBuf::from(value);
                    if !path.is_absolute() {
                        return Err("Use an absolute invitation file path".into());
                    }
                    api::read_private(&path, 8192)?
                } else {
                    value.into()
                };
                if document.is_empty() || document.len() > 8192 {
                    return Err("Enter an invitation of at most 8192 bytes".into());
                }
                self.modal = None;
                self.send(Work {
                    kind: Kind::Preview(document.clone()),
                    task: Task::Request(json!({"op":"preview_invitation","invitation":document})),
                });
            }
            FormKind::Owner { approve } => {
                let peer = form.fields[0].value.trim();
                peer.parse::<wispnet_core::identity::EndpointId>()
                    .map_err(|_| "Enter the complete client public ID")?;
                let mut action = json!({"op":if approve{"approve"}else{"revoke"},"peer":peer});
                if approve {
                    let name = form.fields[1].value.trim();
                    if !wispnet_core::enrollment::valid_name(name) {
                        return Err("Enter a valid client name".into());
                    }
                    action["name"] = json!(name);
                }
                let targets: Vec<Value> = form
                    .fields
                    .iter()
                    .skip(if approve { 2 } else { 1 })
                    .filter(|f| matches!(f.kind, FieldKind::Check(true)))
                    .map(|f| {
                        serde_json::from_str(&f.value)
                            .map_err(|_| "Invalid owner target".to_string())
                    })
                    .collect::<Result<_, _>>()?;
                if targets.is_empty() {
                    return Err("Select at least one owner target".into());
                }
                let mut detail=vec![format!("Client: {peer}"),"Each selected host acts independently. Offline hosts remain pending. Approval grants no service access automatically.".into()];
                detail.extend(targets.iter().map(|t| {
                    format!(
                        "{} · {}",
                        if text(t, "ssh").is_empty() {
                            "This host"
                        } else {
                            text(t, "ssh")
                        },
                        text(t, "host")
                    )
                }));
                self.confirmation(
                    "Apply across selected owner hosts".into(),
                    detail,
                    Work {
                        kind: Kind::OwnerSaved,
                        task: Task::OwnerStart(json!({"action":action,"targets":targets})),
                    },
                );
            }
        }
        Ok(())
    }
    fn receive(&mut self) {
        while let Ok(reply) = self.rx.try_recv() {
            self.busy = false;
            let old_id = self.entries().get(self.selected).map(|e| e.id.clone());
            match reply.result {
                Err(error) => {
                    if matches!(reply.kind, Kind::Status) {
                        self.online = false;
                    }
                    if let Kind::Catalog(ref peer) = reply.kind {
                        self.catalogs.remove(peer);
                        self.catalog_errors.insert(peer.clone(), error.clone());
                    }
                    self.message = clean(&error);
                    // Do not replay a mutation or silently substitute a newer revision.
                    if !matches!(reply.kind, Kind::Status) {
                        self.modal = Some(Modal::Info {
                            title: "Operation not completed".into(),
                            lines: vec![
                                clean(&error),
                                "Refresh and review the current state before trying again.".into(),
                            ],
                            scroll: 0,
                        });
                        self.last_poll = Instant::now() - Duration::from_secs(5);
                    }
                }
                Ok(value) => {
                    match reply.kind {
                        Kind::Status => {
                            self.snapshot = Some(value);
                            self.online = true;
                            self.names = self.api.names();
                            if self.message.starts_with("Working")
                                || self.message.starts_with("Connecting")
                            {
                                self.message = "Ready".into();
                            }
                        }
                        Kind::Saved => {
                            self.message = "Saved. Refreshing node state…".into();
                            self.last_poll = Instant::now() - Duration::from_secs(5);
                        }
                        Kind::Qr => {
                            self.qr = Some(value);
                            self.page = Page::Qr;
                            self.menu = 1;
                            self.menu_focus = false;
                            self.message="Scan with Wispnet on your phone. Requests will appear automatically.".into();
                        }
                        Kind::Catalog(peer) => {
                            self.catalog_errors.remove(&peer);
                            self.catalogs.insert(peer.clone(), value.clone());
                            let services = array(&value["services"]);
                            let entries = services
                                .iter()
                                .map(|s| {
                                    entry(
                                        text(s, "id"),
                                        format!(
                                            "{} · {}",
                                            clean(text(s, "display_name")),
                                            text(s, "kind")
                                        ),
                                        Action::Remote(peer.clone(), s.clone()),
                                    )
                                })
                                .collect();
                            self.choice("Remote services",vec![self.name(&peer),format!("Full host ID: {peer}"),if services.is_empty(){"No services granted here, or request still awaits approval on the target host.".into()}else{"These are the services this host currently permits you to access.".into()}],entries);
                            self.message = "Authenticated catalog refreshed".into();
                        }
                        Kind::Preview(document) => {
                            let detail=vec![format!("Host: {}",clean(text(&value,"host_name"))),format!("Compare full host ID: {}",text(&value,"host")),format!("Relay: {}",clean(text(&value,"relay_url"))),"Compare this ID with the target computer before confirming. The target owner must approve this computer and grant services.".into()];
                            self.confirmation(
                                "Request pairing with this host".into(),
                                detail,
                                Work {
                                    kind: Kind::Paired,
                                    task: Task::Pair {
                                        preview: value,
                                        document,
                                    },
                                },
                            );
                        }
                        Kind::Paired => {
                            self.names = self.api.names();
                            self.catalogs.remove(text(&value, "host"));
                            self.message="Request sent. Approve this computer on the target host, then grant services.".into();
                            self.last_poll = Instant::now() - Duration::from_secs(5);
                        }
                        Kind::Open => {
                            self.modal=Some(Modal::Info{title:"Browser service".into(),lines:vec![text(&value,"url").into(),"This URL belongs to this computer. Over SSH, use a browser on this computer or forward its local gateway port; it is not a public URL.".into(),"On a graphical session the browser opens automatically. TCP services use the native tunnel command.".into()],scroll:0});
                            self.message = "Browser route ready".into();
                            self.last_poll = Instant::now() - Duration::from_secs(5);
                        }
                        Kind::Owners => {
                            self.operations = array(&value);
                            self.message = "Owner journals loaded".into();
                        }
                        Kind::OwnerSaved => {
                            self.operations.retain(|v| v["id"] != value["id"]);
                            self.operations.push(value.clone());
                            self.action(Action::OwnerDetail(value));
                            self.message = "Review each host's acknowledgement".into();
                            self.last_poll = Instant::now() - Duration::from_secs(5);
                        }
                    }
                }
            }
            if let Some(id) = old_id {
                if let Some(index) = self.entries().iter().position(|e| e.id == id) {
                    self.selected = index;
                } else {
                    self.selected = self.selected.min(self.entries().len().saturating_sub(1));
                }
            }
        }
        if !self.busy
            && let Some(work) = self.pending_work.take()
        {
            self.send(work);
        }
        if let Some(qr) = &self.qr {
            let pending = self
                .pending()
                .into_iter()
                .find(|v| v["invitation"] == qr["invitation"]);
            if let Some(pending) = pending {
                self.qr = None;
                self.navigate(Page::Pending);
                self.selected = self
                    .pending()
                    .iter()
                    .position(|v| v["invitation"] == pending["invitation"])
                    .unwrap_or(0);
                self.message =
                    "Pairing request received. Select the device and press Enter to review.".into();
            } else if array(&self.status()["invitations"]).iter().any(|v| {
                v["invitation"] == qr["invitation"]
                    && !matches!(text(v, "state"), "issued" | "challenged")
            }) {
                self.qr = None;
                self.message = "Invitation is no longer active. Press g for a new QR.".into();
            } else if qr["expires_at"].as_u64().unwrap_or(0) <= now() {
                self.qr = None;
                self.message = "QR expired. Press g to generate another.".into();
            }
        }
        if !self.busy && self.last_poll.elapsed() >= Duration::from_secs(3) {
            self.last_poll = Instant::now();
            self.send(Work {
                kind: Kind::Status,
                task: Task::Status,
            });
        }
    }
    fn key(&mut self, key: KeyEvent) -> bool {
        if key.kind == KeyEventKind::Release {
            return false;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return true;
        }
        if let Some(mut modal) = self.modal.take() {
            if key.code == KeyCode::Esc {
                return false;
            }
            match &mut modal {
                Modal::Info { scroll, .. } => match key.code {
                    KeyCode::Down | KeyCode::PageDown => *scroll = scroll.saturating_add(1),
                    KeyCode::Up | KeyCode::PageUp => *scroll = scroll.saturating_sub(1),
                    KeyCode::Enter | KeyCode::Char('q') => return false,
                    _ => {}
                },
                Modal::Choice {
                    entries,
                    selected,
                    scroll,
                    ..
                } => {
                    let len = entries.len();
                    match key.code {
                        KeyCode::PageDown => *scroll = scroll.saturating_add(3),
                        KeyCode::PageUp => *scroll = scroll.saturating_sub(3),
                        KeyCode::Down | KeyCode::Tab => *selected = (*selected + 1) % len.max(1),
                        KeyCode::Up | KeyCode::BackTab => {
                            *selected = (*selected + len.saturating_sub(1)) % len.max(1)
                        }
                        KeyCode::Enter => {
                            if let Some(row) = entries.get(*selected) {
                                self.action(row.action.clone());
                            }
                            return false;
                        }
                        _ => {}
                    }
                }
                Modal::Form(form) => {
                    let count = form.fields.len() + 1;
                    match key.code {
                        KeyCode::Down | KeyCode::Tab => form.selected = (form.selected + 1) % count,
                        KeyCode::Up | KeyCode::BackTab => {
                            form.selected = (form.selected + count - 1) % count
                        }
                        KeyCode::Enter if form.selected == form.fields.len() => {
                            let saved = form.clone();
                            if let Err(error) = self.save_form(saved) {
                                self.message = error;
                                self.modal = Some(modal);
                            }
                            return false;
                        }
                        _ => {
                            if let Some(field) = form.fields.get_mut(form.selected) {
                                match &mut field.kind {
                                    FieldKind::Check(v) => {
                                        if matches!(key.code, KeyCode::Enter | KeyCode::Char(' ')) {
                                            *v = !*v;
                                        }
                                    }
                                    FieldKind::Choice(choices, index) => match key.code {
                                        KeyCode::Enter | KeyCode::Right | KeyCode::Char(' ') => {
                                            *index = (*index + 1) % choices.len()
                                        }
                                        KeyCode::Left => {
                                            *index = (*index + choices.len() - 1) % choices.len()
                                        }
                                        _ => {}
                                    },
                                    FieldKind::Text(limit) => match key.code {
                                        KeyCode::Char('u')
                                            if key.modifiers.contains(KeyModifiers::CONTROL) =>
                                        {
                                            field.value.clear()
                                        }
                                        KeyCode::Char(c)
                                            if !key.modifiers.intersects(
                                                KeyModifiers::CONTROL | KeyModifiers::ALT,
                                            ) && field.value.len() + c.len_utf8() <= *limit =>
                                        {
                                            field.value.push(c)
                                        }
                                        KeyCode::Backspace => {
                                            field.value.pop();
                                        }
                                        KeyCode::Enter => {
                                            form.selected = (form.selected + 1) % count
                                        }
                                        _ => {}
                                    },
                                }
                            }
                        }
                    }
                }
            }
            self.modal = Some(modal);
            return false;
        }
        match key.code {
            KeyCode::Char('q') => {
                if self.busy {
                    self.message="Operation in progress. Wait for its result, or Ctrl+C to exit with an unconfirmed result.".into();
                } else {
                    return true;
                }
            }
            KeyCode::Esc => {
                self.qr = None;
                self.navigate(Page::Home);
            }
            KeyCode::Tab | KeyCode::BackTab => self.menu_focus = !self.menu_focus,
            KeyCode::Char('r') if !self.busy => {
                self.last_poll = Instant::now() - Duration::from_secs(5);
            }
            KeyCode::Char('g') => self.action(Action::Qr),
            KeyCode::Char('p') => self.navigate(Page::Pending),
            KeyCode::Char(c @ '1'..='8') => self.navigate(Page::at(c as usize - '1' as usize)),
            KeyCode::Down | KeyCode::Char('j') => {
                if self.menu_focus {
                    self.menu = (self.menu + 1) % 8;
                } else {
                    self.selected = (self.selected + 1) % self.entries().len().max(1);
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.menu_focus {
                    self.menu = (self.menu + 7) % 8;
                } else {
                    let n = self.entries().len().max(1);
                    self.selected = (self.selected + n - 1) % n;
                }
            }
            KeyCode::Enter => {
                if self.menu_focus {
                    self.navigate(Page::at(self.menu));
                } else if let Some(row) = self.entries().get(self.selected) {
                    self.action(row.action.clone());
                }
            }
            _ => {}
        }
        false
    }
    fn paste(&mut self, value: &str) {
        if let Some(Modal::Form(form)) = &mut self.modal
            && let Some(field) = form.fields.get_mut(form.selected)
            && let FieldKind::Text(limit) = field.kind
        {
            if field.value.len() + value.len() <= limit {
                field.value.push_str(value);
            } else {
                self.message = format!("Input exceeds {limit} bytes");
            }
        }
    }
    fn mouse(&mut self, event: event::MouseEvent) {
        let found = self
            .hits
            .iter()
            .rev()
            .find(|(r, _)| r.contains((event.column, event.row).into()))
            .map(|(_, h)| h.clone());
        if event.kind == MouseEventKind::ScrollDown {
            self.key(KeyCode::Down.into());
            return;
        }
        if event.kind == MouseEventKind::ScrollUp {
            self.key(KeyCode::Up.into());
            return;
        }
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        match found {
            Some(Hit::Nav(n)) if self.modal.is_none() => {
                self.navigate(Page::at(n));
            }
            Some(Hit::Row(n)) if self.modal.is_none() => {
                self.menu_focus = false;
                self.selected = n;
                let double = self.last_click.is_some_and(|(when, row)| {
                    row == n && when.elapsed() < Duration::from_millis(400)
                });
                self.last_click = Some((Instant::now(), n));
                if double {
                    self.key(KeyCode::Enter.into());
                }
            }
            Some(Hit::ModalRow(n)) => {
                if let Some(Modal::Choice { selected, .. }) = &mut self.modal {
                    *selected = n;
                }
                self.key(KeyCode::Enter.into());
            }
            Some(Hit::Field(n)) => {
                let toggle = if let Some(Modal::Form(form)) = &mut self.modal {
                    form.selected = n;
                    n == form.fields.len()
                        || form
                            .fields
                            .get(n)
                            .is_some_and(|f| !matches!(f.kind, FieldKind::Text(_)))
                } else {
                    false
                };
                if toggle {
                    self.key(KeyCode::Enter.into());
                }
            }
            _ => {}
        }
    }
    fn draw(&mut self, frame: &mut Frame) {
        self.hits.clear();
        let area = frame.area();
        let base = Style::default().fg(Color::Gray).bg(Color::Rgb(10, 16, 29));
        frame.render_widget(Block::default().style(base), area);
        if area.width < 65 || area.height < 16 {
            frame.render_widget(Paragraph::new("Wispnet — enlarge this terminal to at least 65 columns × 16 rows.\nq: exit · Ctrl+C: exit immediately").wrap(Wrap{trim:false}),area);
            return;
        }
        let parts = Layout::vertical([
            Constraint::Length(4),
            Constraint::Min(6),
            Constraint::Length(3),
        ])
        .split(area);
        let status = self.status();
        let host = self.host();
        let heading = vec![
            Line::from(vec![
                Span::styled(
                    " WISPNET ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(
                    "{} · {}",
                    clean(text(&status, "host_name")),
                    short(text(&host, "host"))
                )),
            ]),
            Line::raw(format!(
                " {} · relay {} · {} pending · {} clients · {} services{}",
                if self.online { "ONLINE" } else { "OFFLINE" },
                text(&status, "relay"),
                self.pending().len(),
                self.clients().len(),
                self.services().len(),
                if self.busy { " · working…" } else { "" }
            )),
        ];
        frame.render_widget(
            Paragraph::new(heading)
                .block(Block::bordered().border_style(Style::default().fg(Color::DarkGray))),
            parts[0],
        );
        if self.page == Page::Qr {
            self.draw_qr(frame, parts[1]);
        } else {
            let body =
                Layout::horizontal([Constraint::Length(26), Constraint::Min(20)]).split(parts[1]);
            let menu_items: Vec<ListItem> = PAGES
                .iter()
                .enumerate()
                .map(|(n, s)| ListItem::new(format!("{} {s}", n + 1)))
                .collect();
            let mut menu_state = ListState::default().with_selected(Some(self.menu));
            let block = Block::bordered()
                .title(if self.menu_focus {
                    " Menu · Enter to open "
                } else {
                    " Menu · Tab "
                })
                .border_style(Style::default().fg(if self.menu_focus {
                    Color::Cyan
                } else {
                    Color::DarkGray
                }));
            frame.render_stateful_widget(
                List::new(menu_items)
                    .block(block)
                    .highlight_style(Style::default().bg(Color::Rgb(31, 55, 84)).fg(Color::White))
                    .highlight_symbol("› "),
                body[0],
                &mut menu_state,
            );
            for n in 0..8 {
                self.hits.push((
                    Rect::new(
                        body[0].x + 1,
                        body[0].y + 1 + n as u16,
                        body[0].width - 2,
                        1,
                    ),
                    Hit::Nav(n),
                ));
            }
            let content =
                Layout::vertical([Constraint::Min(3), Constraint::Length(6)]).split(body[1]);
            let entries = self.entries();
            self.selected = self.selected.min(entries.len().saturating_sub(1));
            self.list_state.select(if entries.is_empty() {
                None
            } else {
                Some(self.selected)
            });
            let items: Vec<ListItem> = entries
                .iter()
                .map(|e| ListItem::new(clean(&e.label)))
                .collect();
            let mut title = PAGES[self.page.index()].to_owned();
            if self.page == Page::Pending {
                title.push_str(" · Enter to review");
            }
            let block =
                Block::bordered()
                    .title(format!(" {title} "))
                    .border_style(Style::default().fg(if self.menu_focus {
                        Color::DarkGray
                    } else {
                        Color::Cyan
                    }));
            if entries.is_empty() {
                frame.render_widget(
                    Paragraph::new(if self.page == Page::Pending {
                        "No pending pairing requests. Press g to generate a QR invitation."
                    } else {
                        "No machines paired. Generate a QR or pair a remote host."
                    })
                    .wrap(Wrap { trim: false })
                    .block(block),
                    content[0],
                );
            } else {
                frame.render_stateful_widget(
                    List::new(items)
                        .block(block)
                        .highlight_symbol("› ")
                        .highlight_style(
                            Style::default().fg(Color::White).bg(Color::Rgb(31, 55, 84)),
                        ),
                    content[0],
                    &mut self.list_state,
                );
            }
            for n in self.list_state.offset()
                ..entries.len().min(
                    self.list_state.offset() + usize::from(content[0].height.saturating_sub(2)),
                )
            {
                self.hits.push((
                    Rect::new(
                        content[0].x + 1,
                        content[0].y + 1 + (n - self.list_state.offset()) as u16,
                        content[0].width - 2,
                        1,
                    ),
                    Hit::Row(n),
                ));
            }
            let details=entries.get(self.selected).map(|v|v.detail.join("\n")).filter(|s|!s.is_empty()).unwrap_or_else(||"↑/↓ select · Enter review/open · double-click a row\nApprovals and removals always show the full device ID before confirmation.".into());
            frame.render_widget(
                Paragraph::new(details).wrap(Wrap { trim: false }).block(
                    Block::bordered()
                        .title(" Details ")
                        .border_style(Style::default().fg(Color::DarkGray)),
                ),
                content[1],
            );
        }
        frame.render_widget(Paragraph::new(vec![Line::raw(clean(&self.message)),Line::styled("Tab: switch pane · ↑↓/Enter: select · g: QR · p: pending · r: refresh · Esc: back · q: quit",Style::default().fg(Color::Cyan))]).wrap(Wrap{trim:false}),parts[2]);
        if self.modal.is_some() {
            self.draw_modal(frame, area);
        }
    }
    fn draw_qr(&self, frame: &mut Frame, area: Rect) {
        let block = Block::bordered().title(" Pair with this computer · g: new QR · Esc: back ");
        let inside = block.inner(area);
        frame.render_widget(block, area);
        let Some(qr) = &self.qr else {
            frame.render_widget(
                Paragraph::new(if self.busy {
                    "Generating invitation…"
                } else {
                    "Press g to generate a five-minute pairing QR."
                }),
                inside,
            );
            return;
        };
        let matrix = array(&qr["matrix"]);
        let width = matrix.len();
        let height = width.div_ceil(2);
        if usize::from(inside.width) < width || usize::from(inside.height) < height + 5 {
            frame.render_widget(Paragraph::new(format!("Enlarge the terminal or reduce its font size.\nThis QR needs at least {} columns × {} rows.\nThe code will not be drawn while it would wrap or clip.\nPress Esc to go back.",width+2,height+14)).wrap(Wrap{trim:false}),inside);
            return;
        }
        let mut lines = vec![
            Line::raw("In Wispnet on your phone, choose Scan pairing QR."),
            Line::raw(format!("Host: {}", text(qr, "host"))),
            Line::raw(format!(
                "Expires in {} seconds · keep this QR private",
                qr["expires_at"].as_u64().unwrap_or(0).saturating_sub(now())
            )),
        ];
        for pair in matrix.chunks(2) {
            let top = array(&pair[0]);
            let bottom = pair.get(1).map(array).unwrap_or_default();
            let text: String = top
                .iter()
                .enumerate()
                .map(
                    |(x, p)| match (p == true, bottom.get(x) == Some(&Value::Bool(true))) {
                        (true, true) => '█',
                        (true, false) => '▀',
                        (false, true) => '▄',
                        (false, false) => ' ',
                    },
                )
                .collect();
            lines.push(Line::from(Span::styled(
                text,
                Style::default().fg(Color::Black).bg(Color::White),
            )));
        }
        lines.push(Line::raw(
            "After scanning, select the incoming request and approve it.",
        ));
        frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), inside);
    }
    fn draw_modal(&mut self, frame: &mut Frame, area: Rect) {
        let rect = Rect::new(
            area.x + 3,
            area.y + 2,
            area.width.saturating_sub(6),
            area.height.saturating_sub(4),
        );
        frame.render_widget(Clear, rect);
        let modal = self.modal.as_ref().unwrap();
        let title = match modal {
            Modal::Choice { title, .. } | Modal::Info { title, .. } => title.clone(),
            Modal::Form(form) => form.title.clone(),
        };
        let block = Block::bordered()
            .title(format!(" {title} · Esc: cancel/back "))
            .border_style(Style::default().fg(Color::Cyan))
            .style(Style::default().bg(Color::Rgb(12, 21, 39)).fg(Color::White));
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        match modal {
            Modal::Info { lines, scroll, .. } => {
                let text = lines
                    .iter()
                    .map(|s| clean(s))
                    .collect::<Vec<_>>()
                    .join("\n\n");
                frame.render_widget(
                    Paragraph::new(text)
                        .wrap(Wrap { trim: false })
                        .scroll((*scroll, 0)),
                    inner,
                );
            }
            Modal::Choice {
                detail,
                entries,
                selected,
                scroll,
                ..
            } => {
                let choice_height = (entries.len() as u16 + 2).min(inner.height / 2).max(2);
                let areas =
                    Layout::vertical([Constraint::Min(2), Constraint::Length(choice_height)])
                        .split(inner);
                frame.render_widget(
                    Paragraph::new(
                        detail
                            .iter()
                            .map(|s| clean(s))
                            .collect::<Vec<_>>()
                            .join("\n\n"),
                    )
                    .wrap(Wrap { trim: false })
                    .scroll((*scroll, 0)),
                    areas[0],
                );
                let mut state = ListState::default().with_selected(if entries.is_empty() {
                    None
                } else {
                    Some(*selected)
                });
                frame.render_stateful_widget(
                    List::new(
                        entries
                            .iter()
                            .map(|e| ListItem::new(clean(&e.label)))
                            .collect::<Vec<_>>(),
                    )
                    .block(
                        Block::default()
                            .borders(Borders::TOP)
                            .title(" Choose · Enter/click · PgUp/PgDn: details "),
                    )
                    .highlight_symbol("› ")
                    .highlight_style(Style::default().bg(Color::Rgb(31, 55, 84)).fg(Color::Cyan)),
                    areas[1],
                    &mut state,
                );
                for n in state.offset()
                    ..entries
                        .len()
                        .min(state.offset() + usize::from(areas[1].height.saturating_sub(1)))
                {
                    self.hits.push((
                        Rect::new(
                            areas[1].x,
                            areas[1].y + 1 + (n - state.offset()) as u16,
                            areas[1].width,
                            1,
                        ),
                        Hit::ModalRow(n),
                    ));
                }
            }
            Modal::Form(form) => {
                let mut items: Vec<ListItem> = form
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(n, f)| {
                        let value = match &f.kind {
                            FieldKind::Text(_) => {
                                if matches!(form.kind, FormKind::Pair { file: false })
                                    && !f.value.is_empty()
                                {
                                    format!("[private invitation: {} bytes]", f.value.len())
                                } else {
                                    clean(&f.value)
                                }
                            }
                            FieldKind::Check(v) => if *v { "[x]" } else { "[ ]" }.into(),
                            FieldKind::Choice(choices, index) => {
                                format!("{}  (←/→ to change)", choices[*index])
                            }
                        };
                        let marker = if form.selected == n { " > " } else { "   " };
                        ListItem::new(vec![
                            Line::styled(
                                format!("{marker}{}", f.label),
                                Style::default().fg(Color::Cyan),
                            ),
                            Line::raw(format!("   {value}")),
                        ])
                    })
                    .collect();
                items.push(ListItem::new(vec![
                    Line::raw(""),
                    Line::styled("   [ Review and save ]", Style::default().fg(Color::Green)),
                ]));
                let sections =
                    Layout::vertical([Constraint::Min(2), Constraint::Length(2)]).split(inner);
                let mut state = ListState::default().with_selected(Some(form.selected));
                frame.render_stateful_widget(
                    List::new(items).highlight_style(Style::default().bg(Color::Rgb(31, 55, 84))),
                    sections[0],
                    &mut state,
                );
                for n in state.offset()
                    ..(form.fields.len() + 1)
                        .min(state.offset() + usize::from(sections[0].height) / 2)
                {
                    self.hits.push((
                        Rect::new(
                            sections[0].x,
                            sections[0].y + ((n - state.offset()) * 2) as u16,
                            sections[0].width,
                            2,
                        ),
                        Hit::Field(n),
                    ));
                }
                frame.render_widget(Paragraph::new("Tab/↑↓: next field · Space: checkbox · Enter: next/review · Ctrl+U: clear\nPaste supported. No capability is written to a history file."),sections[1]);
            }
        }
    }
}
struct Restore;
impl Drop for Restore {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), DisableMouseCapture, DisableBracketedPaste);
        ratatui::restore();
    }
}
pub fn run(config: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err("Interactive management requires a terminal. Use ssh -t desktop.home wispnet, or --help for scripting commands.".into());
    }
    let api = Api::new(config)?;
    let mut app = App::new(api);
    let stop = Arc::new(AtomicBool::new(false));
    let sigterm = signal_hook::flag::register(signal_hook::consts::SIGTERM, stop.clone())?;
    let sigint = signal_hook::flag::register(signal_hook::consts::SIGINT, stop.clone())?;
    let mut terminal = ratatui::init();
    let _restore = Restore;
    execute!(io::stdout(), EnableMouseCapture, EnableBracketedPaste)?;
    while !stop.load(Ordering::Relaxed) {
        app.receive();
        // A QR needs explicit black-on-white contrast even with NO_COLOR set.
        crossterm::style::force_color_output(
            (app.page == Page::Qr && app.qr.is_some()) || std::env::var_os("NO_COLOR").is_none(),
        );
        terminal.draw(|frame| app.draw(frame))?;
        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if app.key(key) => {
                    break;
                }
                Event::Mouse(mouse) => app.mouse(mouse),
                Event::Paste(text) => app.paste(&text),
                _ => {}
            }
        }
    }
    signal_hook::low_level::unregister(sigterm);
    signal_hook::low_level::unregister(sigint);
    Ok(())
}

#[cfg(test)]
mod tests;
