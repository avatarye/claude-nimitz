//! Create and edit forms (plan-v3 5.4): one per thing, fields validated as you
//! type (names locally with core's own rules, everything else by asking core
//! for the plan), the exact `nimitz` command and the plan steps shown below.
//! Long text (prompts, briefs) goes to `$EDITOR` with ctrl-e.

use std::collections::BTreeMap;

use nimitz_core::env::Env;
use nimitz_core::{Caller, Comms, Id, Plan, ShipDef, Snapshot};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::actions::{quote, Act, Pending};

#[derive(Clone, Debug, PartialEq)]
pub enum FormKind {
    NewFleet,
    NewSquadron,
    NewShip,
    EditShip(Id),
    EditFleet(String),
    EditSquadron(Id),
    /// a tender for this commodore
    Tender(Id),
    /// remove a machine label from this fleet
    MachineRm(String),
    /// adopt the stowaway with this session id
    Adopt(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum FieldKind {
    Text,
    /// shown, not editable (context such as the fleet a squadron goes in)
    Fixed,
    /// left / right cycle through the options
    Choice(Vec<String>),
    /// long text: typed inline or edited in `$EDITOR` (ctrl-e)
    Long,
}

#[derive(Clone, Debug)]
pub struct Field {
    pub key: &'static str,
    pub label: &'static str,
    pub value: String,
    pub kind: FieldKind,
    pub hint: &'static str,
    pub error: Option<String>,
}

impl Field {
    fn new(key: &'static str, label: &'static str, value: &str, kind: FieldKind, hint: &'static str) -> Field {
        Field { key, label, value: value.to_string(), kind, hint, error: None }
    }
}

#[derive(Clone, Debug)]
pub struct Form {
    pub kind: FormKind,
    pub title: String,
    pub fields: Vec<Field>,
    pub focus: usize,
    /// the ship as it is on disk (edit), so fields not on the form survive
    pub base: Option<ShipDef>,
    /// refusals and warnings from core's plan, as text
    pub problems: Vec<String>,
    pub warnings: Vec<String>,
    /// the `nimitz` command this form runs
    pub command: String,
    pub plan: Plan,
}

/// What a key did to the form.
#[derive(Clone, Debug, PartialEq)]
pub enum FormKey {
    Stay,
    Cancel,
    Submit,
    /// open `$EDITOR` on the focused field
    Editor,
}

pub const MODES: [&str; 6] = ["(squadron default)", "default", "acceptEdits", "auto", "bypassPermissions", "plan"];
/// For edits: keep what the config has, or clear it.
pub const SET_MODES: [&str; 7] = ["(unchanged)", "(none)", "default", "acceptEdits", "auto", "bypassPermissions", "plan"];

/// `open`, `commodore-only` or `allow:F/SQ/SHIP,...` (the CLI's `--comms`).
pub fn parse_comms(s: &str) -> Result<Comms, String> {
    match s.trim() {
        t @ ("open" | "commodore-only") => Ok(Comms::Mode(t.into())),
        t => match t.strip_prefix("allow:") {
            Some(list) => Ok(Comms::Allow { allow: list.split(',').map(str::trim).filter(|x| !x.is_empty()).map(str::to_string).collect() }),
            None => Err("open, commodore-only or allow:F/SQ/SHIP,...".into()),
        },
    }
}

pub fn comms_text(c: &Comms) -> String {
    match c {
        Comms::Mode(m) => m.clone(),
        Comms::Allow { allow } => format!("allow:{}", allow.join(",")),
    }
}

/// `(unchanged)` -> None, `(none)` -> Some("") (core clears it), else the mode.
fn set_mode(v: &str) -> Option<&str> {
    match v {
        "(unchanged)" => None,
        "(none)" => Some(""),
        m => Some(m),
    }
}

fn machine_label_default(env: &Env) -> String {
    let h: String = env.hostname.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' }).collect();
    if h.is_empty() {
        "this-machine".into()
    } else {
        h
    }
}

impl Form {
    fn build(kind: FormKind, title: String, fields: Vec<Field>) -> Form {
        let focus = fields.iter().position(|f| f.kind != FieldKind::Fixed).unwrap_or(0);
        Form { kind, title, fields, focus, base: None, problems: vec![], warnings: vec![], command: String::new(), plan: Plan::default() }
    }

    pub fn new_fleet(env: &Env) -> Form {
        Form::build(
            FormKind::NewFleet,
            "New fleet".into(),
            vec![
                Field::new("fleet", "fleet", "", FieldKind::Text, "UPPERCASE, A-Z 0-9 _"),
                Field::new("machine", "this machine", &machine_label_default(env), FieldKind::Text, "label for this machine, a-z 0-9 . _ -"),
            ],
        )
    }

    pub fn new_squadron(fleet: &str) -> Form {
        Form::build(
            FormKind::NewSquadron,
            format!("New squadron in {fleet}"),
            vec![
                Field::new("fleet", "fleet", fleet, FieldKind::Fixed, ""),
                Field::new("squadron", "squadron", "", FieldKind::Text, "UPPERCASE, unique in the fleet"),
                Field::new("comms", "comms", "open", FieldKind::Text, "open, commodore-only or allow:F/SQ/SHIP,..."),
                Field::new("brief", "brief", "", FieldKind::Long, "goes to SQUADRON.md; ctrl-e opens $EDITOR"),
            ],
        )
    }

    pub fn new_ship(fleet: &str, sq: &str, machine: &str) -> Form {
        Form::build(
            FormKind::NewShip,
            format!("New ship in {fleet}/{sq}"),
            vec![
                Field::new("fleet", "squadron", &format!("{fleet}/{sq}"), FieldKind::Fixed, ""),
                Field::new("ship", "ship", "", FieldKind::Text, "UPPERCASE, unique in the squadron"),
                Field::new("machine", "machine", machine, FieldKind::Text, "machine label from fleet.toml"),
                Field::new("dir", "folder", "managed", FieldKind::Text, "project folder on that machine, or `managed`"),
                Field::new("prompt", "prompt", "", FieldKind::Long, "what the ship does; ctrl-e opens $EDITOR"),
                Field::new("mode", "permission", MODES[0], FieldKind::Choice(MODES.iter().map(|s| s.to_string()).collect()), "permission mode on that machine"),
                Field::new("depends", "depends on", "", FieldKind::Text, "ships started first, comma separated"),
            ],
        )
    }

    /// Edit a ship as it is in its squadron.toml.
    pub fn edit_ship(env: &Env, id: &Id, machine: &str) -> Result<Form, String> {
        let f = nimitz_core::config::load_fleet(env, id.fleet()).map_err(|i| i.message)?;
        let sq = id.squadron().unwrap_or_default();
        let name = id.parts().get(2).map(|s| s.to_string()).unwrap_or_default();
        let def = f
            .squadrons
            .iter()
            .find(|(_, s)| s.squadron == sq)
            .and_then(|(_, s)| s.ships.iter().find(|x| x.name == name))
            .cloned()
            .ok_or_else(|| format!("{id} is not in {sq}'s squadron.toml"))?;
        let mut form = Form::new_ship(id.fleet(), sq, machine);
        form.kind = FormKind::EditShip(id.clone());
        form.title = format!("Edit ship {id}");
        form.set("ship", &name);
        form.fields[1].kind = FieldKind::Fixed;
        form.set("dir", def.dirs.get(machine).map(String::as_str).unwrap_or(""));
        form.set("prompt", def.prompt.as_deref().unwrap_or(""));
        form.set("mode", def.permission_mode.get(machine).map(String::as_str).unwrap_or(MODES[0]));
        form.set("depends", &def.depends_on.join(","));
        form.base = Some(def);
        form.focus = 3;
        Ok(form)
    }

    pub fn edit_fleet(env: &Env, fleet: &str) -> Result<Form, String> {
        let f = nimitz_core::config::load_fleet(env, fleet).map_err(|i| i.message)?;
        let now = f.config.admiral.permission_mode.clone().unwrap_or_else(|| "(none)".into());
        Ok(Form::build(
            FormKind::EditFleet(fleet.to_string()),
            format!("Edit fleet {fleet}"),
            vec![
                Field::new("fleet", "fleet", fleet, FieldKind::Fixed, ""),
                Field::new("now", "admiral mode now", &now, FieldKind::Fixed, ""),
                Field::new("mode", "admiral mode", SET_MODES[0], FieldKind::Choice(SET_MODES.iter().map(|s| s.to_string()).collect()), "takes effect at the admiral's next start"),
            ],
        ))
    }

    pub fn edit_squadron(env: &Env, id: &Id) -> Result<Form, String> {
        let f = nimitz_core::config::load_fleet(env, id.fleet()).map_err(|i| i.message)?;
        let sq = id.squadron().unwrap_or_default();
        let (_, c) = f.squadrons.iter().find(|(_, s)| s.squadron == sq).ok_or_else(|| format!("no squadron {id}"))?;
        let mode_now = c.permission_mode.clone().unwrap_or_else(|| "(none)".into());
        Ok(Form::build(
            FormKind::EditSquadron(id.clone()),
            format!("Edit squadron {id}"),
            vec![
                Field::new("squadron", "squadron", &id.0, FieldKind::Fixed, ""),
                Field::new("comms", "comms", &comms_text(&c.comms), FieldKind::Text, "open, commodore-only or allow:F/SQ/SHIP,..."),
                Field::new("brief", "new brief", "", FieldKind::Long, "empty keeps the brief; ctrl-e opens $EDITOR"),
                Field::new("now", "ship mode now", &mode_now, FieldKind::Fixed, ""),
                Field::new("mode", "ship mode", SET_MODES[0], FieldKind::Choice(SET_MODES.iter().map(|s| s.to_string()).collect()), "default permission mode for its ships"),
            ],
        ))
    }

    pub fn tender(commodore: &Id) -> Form {
        Form::build(
            FormKind::Tender(commodore.clone()),
            format!("New tender for {}", commodore.name()),
            vec![
                Field::new("squadron", "commodore", &commodore.0, FieldKind::Fixed, ""),
                Field::new("task", "task", "", FieldKind::Text, "short name, e.g. fix-ci (becomes ~task)"),
                Field::new("prompt", "prompt", "", FieldKind::Long, "the job; ctrl-e opens $EDITOR"),
            ],
        )
    }

    pub fn machine_rm(fleet: &str, label: &str) -> Form {
        Form::build(
            FormKind::MachineRm(fleet.to_string()),
            format!("Remove a machine from {fleet}"),
            vec![
                Field::new("fleet", "fleet", fleet, FieldKind::Fixed, ""),
                Field::new("machine", "machine label", label, FieldKind::Text, "leaves fleet.toml and every ship's folders; files stay"),
            ],
        )
    }

    pub fn adopt(session_id: &str, short: &str, suggestion: &str) -> Form {
        Form::build(
            FormKind::Adopt(session_id.to_string()),
            format!("Adopt stowaway {short}"),
            vec![
                Field::new("session", "session", short, FieldKind::Fixed, ""),
                Field::new("id", "as identity", suggestion, FieldKind::Text, "F/SQ/SHIP (a ship in the config) or F/SQ/~task"),
            ],
        )
    }

    pub fn get(&self, key: &str) -> &str {
        self.fields.iter().find(|f| f.key == key).map(|f| f.value.as_str()).unwrap_or("")
    }

    pub fn set(&mut self, key: &str, v: &str) {
        if let Some(f) = self.fields.iter_mut().find(|f| f.key == key) {
            f.value = v.to_string();
        }
    }

    pub fn focused(&self) -> &Field {
        &self.fields[self.focus]
    }

    pub fn ok(&self) -> bool {
        self.problems.is_empty() && self.fields.iter().all(|f| f.error.is_none())
    }

    fn ship_def(&self) -> ShipDef {
        let machine = self.get("machine").trim().to_string();
        let mut def = self.base.clone().unwrap_or_default();
        def.name = self.get("ship").trim().to_string();
        let prompt = self.get("prompt").trim_end().to_string();
        if !prompt.is_empty() {
            def.prompt = Some(prompt);
            def.prompt_file = None;
        } else if def.prompt_file.is_none() {
            def.prompt = None;
        }
        let dir = self.get("dir").trim().to_string();
        if dir.is_empty() {
            def.dirs.remove(&machine);
        } else {
            def.dirs.insert(machine.clone(), dir);
        }
        let mode = self.get("mode");
        if mode == MODES[0] {
            def.permission_mode.remove(&machine);
        } else {
            def.permission_mode.insert(machine, mode.to_string());
        }
        def.depends_on = self.get("depends").split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        def
    }

    /// The `nimitz` command for the form as filled in (the CLI's own flags).
    fn command_line(&self) -> String {
        let q = |s: &str| quote(s);
        match &self.kind {
            FormKind::NewFleet => format!("nimitz fleet new {} --machine {}", q(self.get("fleet")), q(self.get("machine"))),
            FormKind::NewSquadron => {
                let mut c = format!("nimitz squadron new {} --comms {}", q(&format!("{}/{}", self.get("fleet"), self.get("squadron"))), q(self.get("comms").trim()));
                if !self.get("brief").trim().is_empty() {
                    c.push_str(&format!(" --brief {}", q(self.get("brief").trim())));
                }
                c
            }
            FormKind::NewShip | FormKind::EditShip(_) => {
                let verb = if matches!(self.kind, FormKind::NewShip) { "new" } else { "set" };
                let id = format!("{}/{}", self.get("fleet"), self.get("ship"));
                let mut c = format!("nimitz ship {verb} {}", q(&id));
                if !self.get("dir").trim().is_empty() {
                    c.push_str(&format!(" --dir {}", q(self.get("dir").trim())));
                }
                c.push_str(&format!(" --machine {}", q(self.get("machine").trim())));
                if !self.get("prompt").trim().is_empty() {
                    c.push_str(&format!(" --prompt {}", q(self.get("prompt").trim_end())));
                }
                if self.get("mode") != MODES[0] {
                    c.push_str(&format!(" --mode {}", q(self.get("mode"))));
                }
                if !self.get("depends").trim().is_empty() {
                    c.push_str(&format!(" --depends {}", q(self.get("depends").trim())));
                }
                c
            }
            FormKind::EditFleet(f) => match set_mode(self.get("mode")) {
                Some(m) => format!("nimitz fleet set {} --admiral-mode {}", q(f), q(m)),
                None => format!("nimitz fleet set {}", q(f)),
            },
            FormKind::EditSquadron(id) => {
                let mut c = format!("nimitz squadron set {} --comms {}", q(&id.0), q(self.get("comms").trim()));
                if !self.get("brief").trim().is_empty() {
                    c.push_str(&format!(" --brief {}", q(self.get("brief").trim())));
                }
                if let Some(m) = set_mode(self.get("mode")) {
                    c.push_str(&format!(" --mode {}", q(m)));
                }
                c
            }
            FormKind::Tender(id) => format!("nimitz tender {} {} {}", q(&id.0), q(self.get("task").trim()), q(self.get("prompt").trim_end())),
            FormKind::MachineRm(f) => format!("nimitz machine rm {} {}", q(f), q(self.get("machine").trim())),
            FormKind::Adopt(_) => format!("nimitz adopt {} {}", q(self.get("session")), q(self.get("id"))),
        }
    }

    /// Check every field and ask core for the plan; fills errors, problems, command, plan.
    pub fn validate(&mut self, env: &Env, snap: &Snapshot) {
        for f in &mut self.fields {
            f.error = None;
        }
        let name_err = |v: &str, what: &str| -> Option<String> {
            if v.trim().is_empty() {
                Some(format!("{what} is required"))
            } else if !nimitz_core::config::valid_name(v.trim()) {
                Some(format!("{what} must be UPPERCASE letters, digits or _"))
            } else {
                None
            }
        };
        let label_err = |v: &str| -> Option<String> {
            if nimitz_core::config::valid_label(v.trim()) {
                None
            } else {
                Some("a machine label is a-z 0-9 . _ -".into())
            }
        };
        let mut errs: BTreeMap<&'static str, String> = BTreeMap::new();
        let plan = match &self.kind {
            FormKind::NewFleet => {
                if let Some(e) = name_err(self.get("fleet"), "fleet") {
                    errs.insert("fleet", e);
                }
                if let Some(e) = label_err(self.get("machine")) {
                    errs.insert("machine", e);
                }
                nimitz_core::config::plan_fleet_new(env, self.get("fleet").trim(), self.get("machine").trim())
            }
            FormKind::NewSquadron => {
                if let Some(e) = name_err(self.get("squadron"), "squadron") {
                    errs.insert("squadron", e);
                }
                let comms = parse_comms(self.get("comms")).unwrap_or_else(|e| {
                    errs.insert("comms", e);
                    Comms::default()
                });
                nimitz_core::config::plan_squadron_new(env, self.get("fleet"), self.get("squadron").trim(), self.get("brief").trim(), comms)
            }
            FormKind::NewShip | FormKind::EditShip(_) => {
                if let Some(e) = name_err(self.get("ship"), "ship") {
                    errs.insert("ship", e);
                }
                if let Some(e) = label_err(self.get("machine")) {
                    errs.insert("machine", e);
                }
                let dir = self.get("dir").trim();
                if !dir.is_empty() && dir != "managed" && !dir.starts_with('/') && !dir.starts_with("~/") {
                    errs.insert("dir", "an absolute folder, ~/..., or `managed`".into());
                }
                if matches!(self.kind, FormKind::NewShip) && self.get("prompt").trim().is_empty() {
                    errs.insert("prompt", "a new ship needs a prompt (ctrl-e for $EDITOR)".into());
                }
                let fleet_sq = self.get("fleet").to_string();
                let (fleet, sq) = fleet_sq.split_once('/').unwrap_or((fleet_sq.as_str(), ""));
                let def = self.ship_def();
                match &self.kind {
                    FormKind::EditShip(id) => nimitz_core::config::plan_ship_set(env, id, def),
                    _ => nimitz_core::config::plan_ship_new(env, fleet, sq, def),
                }
            }
            FormKind::EditFleet(f) => nimitz_core::config::plan_fleet_set(env, f, set_mode(self.get("mode"))),
            FormKind::EditSquadron(id) => {
                let comms = match parse_comms(self.get("comms")) {
                    Ok(c) => Some(c),
                    Err(e) => {
                        errs.insert("comms", e);
                        None
                    }
                };
                let brief = Some(self.get("brief").trim()).filter(|b| !b.is_empty());
                nimitz_core::config::plan_squadron_set(env, id, brief, comms, set_mode(self.get("mode")))
            }
            FormKind::Tender(cid) => {
                let task = self.get("task").trim();
                if task.is_empty() {
                    errs.insert("task", "a tender needs a task name".into());
                } else if task.contains(char::is_whitespace) || task.contains('/') {
                    errs.insert("task", "one word: letters, digits, - or _".into());
                }
                if self.get("prompt").trim().is_empty() {
                    errs.insert("prompt", "a tender needs a prompt (ctrl-e for $EDITOR)".into());
                }
                nimitz_core::sessions::plan_tender(env, snap, cid, task, self.get("prompt").trim_end(), &Caller::User)
            }
            FormKind::MachineRm(f) => {
                if let Some(e) = label_err(self.get("machine")) {
                    errs.insert("machine", e);
                }
                nimitz_core::config::plan_machine_rm(env, f, self.get("machine").trim())
            }
            FormKind::Adopt(sid) => {
                let id = Id(self.get("id").trim().to_string());
                if id.0.is_empty() || id.parts().len() != 3 {
                    errs.insert("id", "F/SQ/SHIP or F/SQ/~task".into());
                }
                nimitz_core::sessions::plan_adopt(env, snap, sid, &id, &Caller::User)
            }
        };
        for (k, e) in errs {
            if let Some(f) = self.fields.iter_mut().find(|f| f.key == k) {
                f.error = Some(e);
            }
        }
        self.problems = plan.refusals.iter().map(|i| fix_text(&i.message, i.fix.as_deref())).collect();
        self.warnings = plan.warnings.iter().map(|i| fix_text(&i.message, i.fix.as_deref())).collect();
        self.command = self.command_line();
        self.plan = plan;
    }

    /// The confirm-dialog plan for this form.
    pub fn pending(&self) -> Pending {
        let act = match self.kind {
            FormKind::Adopt(_) => Act::Adopt,
            FormKind::Tender(_) => Act::Tender,
            _ => Act::Save,
        };
        let config = !matches!(act, Act::Adopt | Act::Tender);
        Pending { act, command: self.command.clone(), plan: self.plan.clone(), config, panes: vec![], external: None }
    }

    /// The word on the confirm dialog's title.
    pub fn verb(&self) -> &'static str {
        match self.kind {
            FormKind::NewFleet | FormKind::NewSquadron | FormKind::NewShip => "create",
            FormKind::EditShip(_) | FormKind::EditFleet(_) | FormKind::EditSquadron(_) => "update",
            FormKind::Adopt(_) => "adopt",
            FormKind::Tender(_) => "start",
            FormKind::MachineRm(_) => "remove",
        }
    }

    pub fn on_key(&mut self, k: KeyEvent) -> FormKey {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let n = self.fields.len();
        let next_editable = |from: usize, dir: i64, fields: &[Field]| -> usize {
            let mut i = from as i64;
            for _ in 0..n {
                i = (i + dir).rem_euclid(n as i64);
                if fields[i as usize].kind != FieldKind::Fixed {
                    return i as usize;
                }
            }
            from
        };
        match (k.code, ctrl) {
            (KeyCode::Esc, _) => return FormKey::Cancel,
            (KeyCode::Char('s'), true) => return FormKey::Submit,
            (KeyCode::Char('e'), true) => return FormKey::Editor,
            (KeyCode::Tab | KeyCode::Down, _) => self.focus = next_editable(self.focus, 1, &self.fields),
            (KeyCode::BackTab | KeyCode::Up, _) => self.focus = next_editable(self.focus, -1, &self.fields),
            (KeyCode::Enter, _) => {
                let last = (0..n).rev().find(|&i| self.fields[i].kind != FieldKind::Fixed).unwrap_or(0);
                if self.focus == last {
                    return FormKey::Submit;
                }
                self.focus = next_editable(self.focus, 1, &self.fields);
            }
            (KeyCode::Char('u'), true) => {
                if !matches!(self.fields[self.focus].kind, FieldKind::Fixed | FieldKind::Choice(_)) {
                    self.fields[self.focus].value.clear();
                }
            }
            (KeyCode::Left | KeyCode::Right, _) => {
                let f = &mut self.fields[self.focus];
                if let FieldKind::Choice(opts) = &f.kind {
                    let i = opts.iter().position(|o| *o == f.value).unwrap_or(0) as i64;
                    let d = if k.code == KeyCode::Right { 1 } else { -1 };
                    f.value = opts[(i + d).rem_euclid(opts.len() as i64) as usize].clone();
                }
            }
            (KeyCode::Char(' '), _) if matches!(self.fields[self.focus].kind, FieldKind::Choice(_)) => {
                return self.on_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
            }
            (KeyCode::Backspace, _) => {
                if matches!(self.fields[self.focus].kind, FieldKind::Text | FieldKind::Long) {
                    self.fields[self.focus].value.pop();
                }
            }
            (KeyCode::Char(c), false) => {
                if matches!(self.fields[self.focus].kind, FieldKind::Text | FieldKind::Long) {
                    self.fields[self.focus].value.push(c);
                }
            }
            _ => {}
        }
        FormKey::Stay
    }
}

fn fix_text(msg: &str, fix: Option<&str>) -> String {
    match fix {
        Some(f) => format!("{msg}  (fix: {f})"),
        None => msg.to_string(),
    }
}
