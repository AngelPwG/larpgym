use crate::worker::Command;
use anyhow::{anyhow, ensure, Context, Result};
use chrono::{Datelike, Days, Months, NaiveDate, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use gym_core::*;
use gym_storage::{Conflict, Snapshot};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, List, ListItem, ListState, Paragraph, Row, Sparkline, Table,
        TableState, Wrap,
    },
    Frame,
};
use std::sync::mpsc;
use uuid::Uuid;

const TABS: [&str; 8] = [
    "Hoy",
    "Peso",
    "Sesiones",
    "Ejercicios",
    "Rutinas",
    "Hábitos",
    "Progreso",
    "Conflictos",
];
const ACCENT: Color = Color::Cyan;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Motion {
    Up,
    Down,
    First,
    Last,
    HalfUp,
    HalfDown,
    PageUp,
    PageDown,
    Back,
    Open,
    NextTab,
    PreviousTab,
    Ignore,
}

/// Decode navigation only outside text entry and explicit confirmations.
fn navigation(key: KeyEvent, pending_g: &mut bool) -> Option<Motion> {
    if key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        *pending_g = false;
        return Some(match key.code {
            KeyCode::Char('d') if key.modifiers == KeyModifiers::CONTROL => Motion::HalfDown,
            KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => Motion::HalfUp,
            KeyCode::Char('f') if key.modifiers == KeyModifiers::CONTROL => Motion::PageDown,
            KeyCode::Char('b') if key.modifiers == KeyModifiers::CONTROL => Motion::PageUp,
            _ => Motion::Ignore,
        });
    }
    if std::mem::take(pending_g) {
        return Some(match key.code {
            KeyCode::Char('g') => Motion::First,
            KeyCode::Char('t') => Motion::NextTab,
            KeyCode::Char('T') => Motion::PreviousTab,
            KeyCode::Esc => Motion::Back,
            _ => Motion::Ignore,
        });
    }
    Some(match key.code {
        KeyCode::Char('g') => {
            *pending_g = true;
            Motion::Ignore
        }
        KeyCode::Char('G') | KeyCode::End => Motion::Last,
        KeyCode::Home => Motion::First,
        KeyCode::Char('j') | KeyCode::Down => Motion::Down,
        KeyCode::Char('k') | KeyCode::Up => Motion::Up,
        KeyCode::Char('h') | KeyCode::Left | KeyCode::Esc => Motion::Back,
        KeyCode::Char('l') | KeyCode::Right | KeyCode::Enter => Motion::Open,
        KeyCode::PageDown => Motion::PageDown,
        KeyCode::PageUp => Motion::PageUp,
        KeyCode::Tab => Motion::NextTab,
        KeyCode::BackTab => Motion::PreviousTab,
        _ => return None,
    })
}

fn motion_index(motion: Motion, selected: usize, count: usize, page: usize) -> usize {
    let last = count.saturating_sub(1);
    let half = (page / 2).max(1);
    match motion {
        Motion::First => 0,
        Motion::Last => last,
        Motion::Down => selected.saturating_add(1),
        Motion::Up => selected.saturating_sub(1),
        Motion::HalfDown => selected.saturating_add(half),
        Motion::HalfUp => selected.saturating_sub(half),
        Motion::PageDown => selected.saturating_add(page.max(1)),
        Motion::PageUp => selected.saturating_sub(page.max(1)),
        _ => selected,
    }
    .min(last)
}

fn modal_inner_size(area: Rect) -> Rect {
    Block::bordered().inner(popup(area, area.height.saturating_sub(2)))
}

fn scroll_text(scroll: &mut u16, motion: Motion, text: &str, inner: Rect) {
    let lines = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .line_count(inner.width);
    let last = lines
        .saturating_sub(inner.height as usize)
        .min(u16::MAX as usize);
    *scroll = motion_index(motion, *scroll as usize, last + 1, inner.height as usize) as u16;
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    Main,
    Session(NaiveDate, Uuid),
    Routine(Uuid),
    Month,
    ExerciseHistory(Uuid),
    HabitHistory(Uuid),
}
#[derive(Clone)]
enum FormKind {
    Date,
    Weight(NaiveDate),
    Exercise,
    Routine,
    Habit,
    Session(NaiveDate, Option<Uuid>),
    Set(NaiveDate, Uuid, usize, Option<usize>),
    Targets(usize),
}
#[derive(Clone)]
pub struct Form {
    title: String,
    kind: FormKind,
    fields: Vec<(String, String)>,
    focus: usize,
    cursor: usize,
    original: Option<Document>,
}
#[derive(Clone)]
pub enum Choice {
    Attendance(Attendance),
    Habit(Uuid, HabitState),
    StartRoutine(Uuid),
    Exercise(Uuid, View),
}
#[derive(Clone)]
pub enum Action {
    Save(Document, Option<Document>),
    Resolve(Conflict, bool),
}
#[derive(Clone)]
pub enum Modal {
    Form(Form),
    Picker {
        title: String,
        items: Vec<(String, Choice)>,
        selected: usize,
        offset: usize,
    },
    Confirm {
        message: String,
        action: Action,
    },
    Conflict {
        value: Conflict,
        scroll: u16,
    },
    Help {
        scroll: u16,
    },
}
pub struct App {
    pub snapshot: Snapshot,
    pub network: String,
    pub message: String,
    pub ready: bool,
    pub saving: bool,
    pub modal: Option<Modal>,
    date: NaiveDate,
    timezone: chrono_tz::Tz,
    tab: usize,
    view: View,
    after_save: Option<View>,
    table: TableState,
    pending_g: bool,
    terminal_area: Rect,
    visible_rows: usize,
}
impl App {
    pub fn new(date: NaiveDate, timezone: chrono_tz::Tz) -> Self {
        Self {
            snapshot: Snapshot::default(),
            network: "Preparando sincronización…".into(),
            message: "Cargando almacenamiento local…".into(),
            ready: false,
            saving: false,
            modal: None,
            date,
            timezone,
            tab: 0,
            view: View::Main,
            after_save: None,
            table: TableState::default().with_selected(0),
            pending_g: false,
            terminal_area: Rect::new(0, 0, 110, 30),
            visible_rows: 20,
        }
    }
    fn index(&self) -> usize {
        self.table.selected().unwrap_or(0)
    }
    fn reset(&mut self) {
        self.table = TableState::default().with_selected(0);
        self.pending_g = false;
    }
    fn document(&self, id: Uuid) -> Option<Document> {
        self.snapshot.documents.iter().find(|d| d.id == id).cloned()
    }
    fn day_doc(&self, date: NaiveDate) -> Document {
        self.document(day_id(date))
            .filter(|d| !d.deleted)
            .unwrap_or_else(|| Document::day(date))
    }
    fn day(&self, date: NaiveDate) -> Day {
        if let Entity::Day(d) = self.day_doc(date).entity {
            d
        } else {
            unreachable!()
        }
    }
    fn docs(&self, kind: &str) -> Vec<Document> {
        let mut docs: Vec<_> = self
            .snapshot
            .documents
            .iter()
            .filter(|d| !d.deleted && d.kind() == kind)
            .cloned()
            .collect();
        docs.sort_by_key(|d| d.name().to_lowercase());
        docs
    }
    fn days(&self) -> Vec<Day> {
        let mut days: Vec<_> = self
            .snapshot
            .documents
            .iter()
            .filter(|d| !d.deleted)
            .filter_map(|d| {
                if let Entity::Day(day) = &d.entity {
                    Some(day.clone())
                } else {
                    None
                }
            })
            .collect();
        days.sort_by_key(|d| std::cmp::Reverse(d.date));
        days
    }
    fn sessions(&self) -> Vec<(NaiveDate, Session)> {
        self.days()
            .into_iter()
            .flat_map(|d| d.sessions.into_iter().rev().map(move |s| (d.date, s)))
            .collect()
    }
    fn today_habits(&self) -> Vec<(Uuid, Habit)> {
        let day = self.day(self.date);
        self.docs("habit")
            .into_iter()
            .filter_map(|d| {
                if let Entity::Habit(h) = d.entity {
                    if (!h.archived
                        && h.weekdays
                            .contains(&(self.date.weekday().number_from_monday() as u8)))
                        || day.habits.contains_key(&d.id)
                    {
                        Some((d.id, h))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect()
    }
    fn form(
        &mut self,
        title: &str,
        kind: FormKind,
        fields: Vec<(&str, String)>,
        original: Option<Document>,
    ) {
        self.message.clear();
        self.modal = Some(Modal::Form(Form {
            title: title.into(),
            kind,
            cursor: fields.first().map_or(0, |(_, v)| v.len()),
            fields: fields.into_iter().map(|(k, v)| (k.into(), v)).collect(),
            focus: 0,
            original,
        }));
    }
    fn weight_form(&mut self, date: NaiveDate) {
        let weight = self.day(date).weight.map(kg).unwrap_or_default();
        self.form(
            "Peso corporal · kg",
            FormKind::Weight(date),
            vec![("Kilogramos (hasta 3 decimales)", weight)],
            self.document(day_id(date)),
        );
    }
    fn commit(
        &mut self,
        document: Document,
        original: Option<Document>,
        tx: &mpsc::Sender<Command>,
    ) -> Result<()> {
        document.validate()?;
        tx.send(Command::Save { document, original })
            .map_err(|_| anyhow!("El trabajador de almacenamiento se cerró"))?;
        self.saving = true;
        self.message = "Guardando en SQLite…".into();
        Ok(())
    }
    fn save(&mut self, document: Document, tx: &mpsc::Sender<Command>) -> Result<()> {
        let original = self.document(document.id);
        self.commit(document, original, tx)
    }
    fn confirm_save(&mut self, message: &str, document: Document) {
        let original = self.document(document.id);
        self.modal = Some(Modal::Confirm {
            message: message.into(),
            action: Action::Save(document, original),
        });
    }
    fn picker(&mut self, title: &str, items: Vec<(String, Choice)>) -> Result<()> {
        ensure!(
            !items.is_empty(),
            "No hay opciones activas. Crea primero un registro en su panel"
        );
        self.modal = Some(Modal::Picker {
            title: title.into(),
            items,
            selected: 0,
            offset: 0,
        });
        Ok(())
    }
    fn attendance_picker(&mut self) -> Result<()> {
        self.picker(
            "Asistencia",
            [
                Attendance::Trained,
                Attendance::Rest,
                Attendance::Missed,
                Attendance::Unrecorded,
            ]
            .into_iter()
            .map(|s| (s.label().into(), Choice::Attendance(s)))
            .collect(),
        )
    }
    fn habit_picker(&mut self, id: Uuid) -> Result<()> {
        self.picker(
            "Estado del hábito",
            [HabitState::Done, HabitState::Pending, HabitState::Skipped]
                .into_iter()
                .map(|s| (s.label().into(), Choice::Habit(id, s)))
                .collect(),
        )
    }
    fn routine_picker(&mut self) -> Result<()> {
        self.picker(
            "Iniciar sesión desde rutina",
            self.docs("routine")
                .into_iter()
                .filter_map(|d| {
                    if let Entity::Routine(r) = d.entity {
                        if !r.archived {
                            Some((r.name, Choice::StartRoutine(d.id)))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .collect(),
        )
    }
    fn exercise_picker(&mut self, target: View) -> Result<()> {
        self.picker(
            "Añadir ejercicio",
            self.docs("exercise")
                .into_iter()
                .filter_map(|d| {
                    if let Entity::Exercise(e) = d.entity {
                        if !e.archived {
                            Some((e.name, Choice::Exercise(d.id, target)))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .collect(),
        )
    }
    fn session_form(&mut self, date: NaiveDate, id: Option<Uuid>) {
        let s = id.and_then(|id| self.day(date).sessions.into_iter().find(|s| s.id == id));
        self.form(
            "Sesión · cada confirmación se guarda",
            FormKind::Session(date, id),
            vec![
                (
                    "Nombre",
                    s.as_ref()
                        .map_or("Entrenamiento".into(), |s| s.name.clone()),
                ),
                ("Notas", s.map_or(String::new(), |s| s.notes)),
            ],
            self.document(day_id(date)),
        );
    }
    fn training_row(&self) -> Option<(usize, Option<usize>)> {
        if let View::Session(date, id) = self.view {
            let s = self.day(date).sessions.into_iter().find(|s| s.id == id)?;
            let mut rows = vec![];
            for (i, e) in s.exercises.iter().enumerate() {
                rows.push((i, None));
                for j in 0..e.sets.len() {
                    rows.push((i, Some(j)));
                }
            }
            rows.get(self.index()).copied()
        } else {
            None
        }
    }
    fn set_form(
        &mut self,
        date: NaiveDate,
        id: Uuid,
        exercise: usize,
        set: Option<usize>,
    ) -> Result<()> {
        let s = self
            .day(date)
            .sessions
            .into_iter()
            .find(|s| s.id == id)
            .context("Sesión inexistente")?;
        let previous = s.exercises[exercise].sets.last();
        let selected = set
            .and_then(|j| s.exercises[exercise].sets.get(j))
            .or(previous);
        self.form(
            "Serie · carga y repeticiones",
            FormKind::Set(date, id, exercise, set),
            vec![
                (
                    "Carga en kg (0 es válido)",
                    selected.map_or("0".into(), |s| kg(s.load)),
                ),
                (
                    "Repeticiones",
                    selected.map_or("10".into(), |s| s.reps.to_string()),
                ),
            ],
            self.document(day_id(date)),
        );
        Ok(())
    }
    fn build_form(&mut self, form: &Form) -> Result<Option<Document>> {
        let field = |i: usize| form.fields[i].1.trim().to_owned();
        let original = form.original.clone();
        let mut doc = match form.kind {
            FormKind::Date => {
                self.date = NaiveDate::parse_from_str(&field(0), "%Y-%m-%d")
                    .context("Fecha inválida; usa AAAA-MM-DD")?;
                self.view = View::Main;
                self.tab = 0;
                self.reset();
                return Ok(None);
            }
            FormKind::Weight(date) | FormKind::Session(date, _) | FormKind::Set(date, _, _, _) => {
                original.clone().unwrap_or_else(|| Document::day(date))
            }
            FormKind::Exercise => original.clone().unwrap_or_else(|| Document {
                id: Uuid::new_v4(),
                deleted: false,
                entity: Entity::Exercise(Exercise {
                    name: String::new(),
                    archived: false,
                }),
            }),
            FormKind::Routine => original.clone().unwrap_or_else(|| Document {
                id: Uuid::new_v4(),
                deleted: false,
                entity: Entity::Routine(Routine {
                    name: String::new(),
                    entries: vec![],
                    archived: false,
                }),
            }),
            FormKind::Habit => original.clone().unwrap_or_else(|| Document {
                id: Uuid::new_v4(),
                deleted: false,
                entity: Entity::Habit(Habit {
                    name: String::new(),
                    weekdays: vec![],
                    reminder: None,
                    archived: false,
                }),
            }),
            FormKind::Targets(_) => original.clone().context("Rutina inexistente")?,
        };
        match (&form.kind, &mut doc.entity) {
            (FormKind::Weight(_), Entity::Day(d)) => d.weight = Some(parse_kg(&field(0))?),
            (FormKind::Exercise, Entity::Exercise(e)) => e.name = field(0),
            (FormKind::Routine, Entity::Routine(r)) => r.name = field(0),
            (FormKind::Habit, Entity::Habit(h)) => {
                h.name = field(0);
                h.weekdays = field(1)
                    .split(',')
                    .map(|s| {
                        s.trim()
                            .parse::<u8>()
                            .context("Días inválidos; usa 1,2,3,4,5,6,7")
                    })
                    .collect::<Result<_>>()?;
                h.reminder = if field(2).is_empty() {
                    None
                } else {
                    Some(field(2))
                };
            }
            (FormKind::Session(date, id), Entity::Day(d)) => {
                if let Some(id) = id {
                    let s = d
                        .sessions
                        .iter_mut()
                        .find(|s| s.id == *id)
                        .context("Sesión inexistente")?;
                    s.name = field(0);
                    s.notes = field(1);
                } else {
                    let id = Uuid::new_v4();
                    d.sessions.push(Session {
                        id,
                        name: field(0),
                        notes: field(1),
                        status: SessionStatus::InProgress,
                        exercises: vec![],
                    });
                    self.after_save = Some(View::Session(*date, id));
                }
            }
            (FormKind::Set(_, id, i, j), Entity::Day(d)) => {
                let s = d
                    .sessions
                    .iter_mut()
                    .find(|s| s.id == *id)
                    .context("Sesión inexistente")?;
                let e = s.exercises.get_mut(*i).context("Ejercicio inexistente")?;
                let load = parse_kg(&field(0))?;
                let reps = field(1)
                    .parse::<u16>()
                    .context("Repeticiones: usa un entero entre 1 y 1000")?;
                if let Some(j) = j {
                    e.sets[*j].load = load;
                    e.sets[*j].reps = reps;
                } else {
                    e.sets.push(Set {
                        id: Uuid::new_v4(),
                        load,
                        reps,
                    });
                }
            }
            (FormKind::Targets(i), Entity::Routine(r)) => {
                r.entries[*i].target_sets = optional_number(&field(0))?;
                r.entries[*i].target_reps = optional_number(&field(1))?;
            }
            _ => return Err(anyhow!("Formulario incompatible")),
        }
        doc.deleted = false;
        doc.validate()?;
        Ok(Some(doc))
    }
    fn choose(&mut self, choice: Choice, tx: &mpsc::Sender<Command>) -> Result<()> {
        match choice {
            Choice::Attendance(state) => {
                let mut doc = self.day_doc(self.date);
                if let Entity::Day(d) = &mut doc.entity {
                    d.attendance = state;
                }
                self.save(doc, tx)?;
            }
            Choice::Habit(id, state) => {
                let mut doc = self.day_doc(self.date);
                if let Entity::Day(d) = &mut doc.entity {
                    d.habits.insert(id, state);
                }
                self.save(doc, tx)?;
            }
            Choice::StartRoutine(id) => {
                let routine = self.document(id).context("Rutina inexistente")?;
                let Entity::Routine(r) = routine.entity else {
                    unreachable!()
                };
                let mut exercises = vec![];
                for entry in r.entries {
                    let document = self
                        .document(entry.exercise_id)
                        .context("Ejercicio de rutina inexistente")?;
                    let Entity::Exercise(e) = document.entity else {
                        unreachable!()
                    };
                    exercises.push(SessionExercise {
                        id: Uuid::new_v4(),
                        exercise_id: entry.exercise_id,
                        name: e.name,
                        target_sets: entry.target_sets,
                        target_reps: entry.target_reps,
                        sets: vec![],
                    });
                }
                let session_id = Uuid::new_v4();
                let mut doc = self.day_doc(self.date);
                if let Entity::Day(d) = &mut doc.entity {
                    d.sessions.push(Session {
                        id: session_id,
                        name: r.name,
                        notes: String::new(),
                        status: SessionStatus::InProgress,
                        exercises,
                    });
                }
                self.after_save = Some(View::Session(self.date, session_id));
                self.save(doc, tx)?;
            }
            Choice::Exercise(id, target) => {
                let e = self.document(id).context("Ejercicio inexistente")?;
                let Entity::Exercise(e) = e.entity else {
                    unreachable!()
                };
                match target {
                    View::Session(date, sid) => {
                        let mut doc = self.day_doc(date);
                        if let Entity::Day(d) = &mut doc.entity {
                            let s = d
                                .sessions
                                .iter_mut()
                                .find(|s| s.id == sid)
                                .context("Sesión inexistente")?;
                            s.exercises.push(SessionExercise {
                                id: Uuid::new_v4(),
                                exercise_id: id,
                                name: e.name,
                                target_sets: None,
                                target_reps: None,
                                sets: vec![],
                            });
                        }
                        self.save(doc, tx)?;
                    }
                    View::Routine(rid) => {
                        let mut doc = self.document(rid).context("Rutina inexistente")?;
                        if let Entity::Routine(r) = &mut doc.entity {
                            r.entries.push(RoutineEntry {
                                exercise_id: id,
                                target_sets: None,
                                target_reps: None,
                            });
                        }
                        self.save(doc, tx)?;
                    }
                    _ => (),
                }
            }
        }
        Ok(())
    }
    pub fn saved(&mut self, result: std::result::Result<Snapshot, String>) {
        self.saving = false;
        match result {
            Ok(snapshot) => {
                self.snapshot = snapshot;
                self.modal = None;
                self.message = "Guardado localmente".into();
                if let Some(view) = self.after_save.take() {
                    self.view = view;
                    self.reset();
                }
            }
            Err(e) => {
                self.after_save = None;
                self.message = format!("No se guardó: {e}");
            }
        }
    }
    pub fn key(
        &mut self,
        key: KeyEvent,
        tx: &mpsc::Sender<Command>,
        sync: &tokio::sync::mpsc::Sender<()>,
    ) -> Result<bool> {
        if self.saving {
            self.message = "Guardando… espera la confirmación local".into();
            return Ok(false);
        }
        let result = self.handle_key(key, tx, sync);
        match result {
            Ok(quit) => Ok(quit),
            Err(e) => {
                self.after_save = None;
                self.message = e.to_string();
                Ok(false)
            }
        }
    }
    fn handle_key(
        &mut self,
        key: KeyEvent,
        tx: &mpsc::Sender<Command>,
        sync: &tokio::sync::mpsc::Sender<()>,
    ) -> Result<bool> {
        if let Some(mut modal) = self.modal.take() {
            if key.code == KeyCode::Esc
                || !matches!(modal, Modal::Form(_))
                    && matches!(key.code, KeyCode::Char('h' | 'q') | KeyCode::Left)
            {
                self.pending_g = false;
                return Ok(false);
            }
            let motion = if matches!(modal, Modal::Form(_) | Modal::Confirm { .. }) {
                self.pending_g = false;
                None
            } else {
                navigation(key, &mut self.pending_g)
            };
            let result = (|| -> Result<bool> {
                match &mut modal {
                    Modal::Form(form) => match key.code {
                        KeyCode::Tab | KeyCode::Down => {
                            form.focus = (form.focus + 1) % form.fields.len();
                            form.cursor = form.fields[form.focus].1.len();
                        }
                        KeyCode::BackTab | KeyCode::Up => {
                            form.focus = (form.focus + form.fields.len() - 1) % form.fields.len();
                            form.cursor = form.fields[form.focus].1.len();
                        }
                        KeyCode::Left => {
                            form.cursor = form.fields[form.focus].1[..form.cursor]
                                .char_indices()
                                .next_back()
                                .map_or(0, |(i, _)| i);
                        }
                        KeyCode::Right => {
                            let value = &form.fields[form.focus].1;
                            form.cursor += value[form.cursor..]
                                .chars()
                                .next()
                                .map_or(0, char::len_utf8);
                        }
                        KeyCode::Home => form.cursor = 0,
                        KeyCode::End => form.cursor = form.fields[form.focus].1.len(),
                        KeyCode::Backspace => {
                            if form.cursor > 0 {
                                let start = form.fields[form.focus].1[..form.cursor]
                                    .char_indices()
                                    .next_back()
                                    .map_or(0, |(i, _)| i);
                                form.fields[form.focus]
                                    .1
                                    .replace_range(start..form.cursor, "");
                                form.cursor = start;
                            }
                        }
                        KeyCode::Delete => {
                            let value = &mut form.fields[form.focus].1;
                            if let Some(c) = value[form.cursor..].chars().next() {
                                value.replace_range(form.cursor..form.cursor + c.len_utf8(), "");
                            }
                        }
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            form.fields[form.focus].1.clear();
                            form.cursor = 0;
                        }
                        KeyCode::Char(c)
                            if !key.modifiers.contains(KeyModifiers::CONTROL)
                                && !c.is_control() =>
                        {
                            let value = &mut form.fields[form.focus].1;
                            if value.len() < 4000 {
                                value.insert(form.cursor, c);
                                form.cursor += c.len_utf8();
                            }
                        }
                        KeyCode::Enter => {
                            if form.focus + 1 < form.fields.len()
                                && !key.modifiers.contains(KeyModifiers::CONTROL)
                            {
                                form.focus += 1;
                                form.cursor = form.fields[form.focus].1.len();
                            } else {
                                if let Some(doc) = self.build_form(form)? {
                                    self.commit(doc, form.original.clone(), tx)?;
                                } else {
                                    return Ok(false);
                                }
                            }
                        }
                        _ => (),
                    },
                    Modal::Picker {
                        items, selected, ..
                    } => match motion {
                        Some(Motion::Open) => {
                            self.choose(items[*selected].1.clone(), tx)?;
                        }
                        Some(motion) => {
                            let page = modal_inner_size(self.terminal_area)
                                .height
                                .saturating_sub(1) as usize;
                            *selected = motion_index(motion, *selected, items.len(), page);
                        }
                        None => (),
                    },
                    Modal::Confirm { action, .. } => match key.code {
                        _ if key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {}
                        KeyCode::Char('s') | KeyCode::Char('S') => match action.clone() {
                            Action::Save(doc, original) => self.commit(doc, original, tx)?,
                            Action::Resolve(conflict, local) => {
                                tx.send(Command::Resolve { conflict, local })?;
                                self.saving = true;
                                self.message = "Guardando resolución…".into();
                            }
                        },
                        KeyCode::Char('n') | KeyCode::Char('N') => return Ok(false),
                        _ => (),
                    },
                    Modal::Conflict { value, scroll } => match key.code {
                        KeyCode::Char('L' | 'S' | 's') if motion.is_none() => {
                            let local = key.code == KeyCode::Char('L');
                            self.modal=Some(Modal::Confirm { message:format!("¿Conservar el día/registro {}? Ambas propuestas quedarán en la auditoría local.",if local { "local" } else { "del servidor" }),action:Action::Resolve(value.clone(),local) });
                            return Ok(false);
                        }
                        _ => {
                            if let Some(motion) = motion {
                                scroll_text(
                                    scroll,
                                    motion,
                                    &conflict_text(value),
                                    modal_inner_size(self.terminal_area),
                                );
                            }
                        }
                    },
                    Modal::Help { scroll } => {
                        if let Some(motion) = motion {
                            scroll_text(scroll, motion, HELP, modal_inner_size(self.terminal_area));
                        }
                    }
                }
                Ok(true)
            })();
            match result {
                Ok(true) => self.modal = Some(modal),
                Ok(false) => (),
                Err(e) => {
                    self.modal = Some(modal);
                    return Err(e);
                }
            }
            return Ok(false);
        }
        let mut key = key;
        if let Some(motion) = navigation(key, &mut self.pending_g) {
            match motion {
                Motion::Open => key.code = KeyCode::Enter,
                Motion::Back => {
                    if self.view != View::Main {
                        self.view = View::Main;
                        self.reset();
                    }
                    return Ok(false);
                }
                Motion::NextTab | Motion::PreviousTab => {
                    self.tab = (self.tab + if motion == Motion::NextTab { 1 } else { 7 }) % 8;
                    self.view = View::Main;
                    self.reset();
                    return Ok(false);
                }
                Motion::Ignore => return Ok(false),
                motion => {
                    let selected =
                        motion_index(motion, self.index(), self.rows().len(), self.visible_rows);
                    self.table.select(Some(selected));
                    return Ok(false);
                }
            }
        }
        match key.code {
            KeyCode::Char('q') => return Ok(true),
            KeyCode::Char('?') | KeyCode::F(1) => {
                self.modal = Some(Modal::Help { scroll: 0 });
                return Ok(false);
            }
            KeyCode::Char('s') => {
                sync.try_send(()).map_err(|_| {
                    anyhow!("Sincronización ya solicitada o sin conexión configurada")
                })?;
                self.message = "Sincronización solicitada".into();
                return Ok(false);
            }
            _ => (),
        }
        if !self.ready {
            return Ok(false);
        }
        match key.code {
            KeyCode::Char(c @ '1'..='8') => {
                self.tab = (c as u8 - b'1') as usize;
                self.view = View::Main;
                self.reset();
                return Ok(false);
            }
            KeyCode::Char('D') => {
                self.form(
                    "Consultar o editar fecha",
                    FormKind::Date,
                    vec![("Fecha local AAAA-MM-DD", self.date.to_string())],
                    None,
                );
                return Ok(false);
            }
            KeyCode::Char('[') | KeyCode::Char(']') => {
                let next = key.code == KeyCode::Char(']');
                self.date = if self.view == View::Month {
                    let first = self.date.with_day(1).unwrap();
                    if next {
                        first.checked_add_months(Months::new(1))
                    } else {
                        first.checked_sub_months(Months::new(1))
                    }
                } else if next {
                    self.date.checked_add_days(Days::new(1))
                } else {
                    self.date.checked_sub_days(Days::new(1))
                }
                .context("Fecha fuera de rango")?;
                if matches!(self.view, View::Session(..)) {
                    self.view = View::Main;
                }
                self.reset();
                return Ok(false);
            }
            _ => (),
        }
        match self.view {
            View::Main => self.main_action(key.code, tx)?,
            View::Session(date, id) => self.session_action(key.code, date, id, tx)?,
            View::Routine(id) => self.routine_action(key.code, id, tx)?,
            View::Month if key.code == KeyCode::Enter => {
                self.date = self
                    .date
                    .with_day(1)
                    .unwrap()
                    .checked_add_days(Days::new(self.index() as u64))
                    .context("Fecha inválida")?;
                self.view = View::Main;
                self.tab = 0;
                self.reset();
            }
            _ => (),
        }
        Ok(false)
    }
    fn main_action(&mut self, key: KeyCode, tx: &mpsc::Sender<Command>) -> Result<()> {
        let index = self.index();
        match self.tab {
            0 => match key {
                KeyCode::Char('w') => self.weight_form(self.date),
                KeyCode::Char('a') => self.attendance_picker()?,
                KeyCode::Char('n') => self.session_form(self.date, None),
                KeyCode::Char('r') => self.routine_picker()?,
                KeyCode::Char('m') => {
                    self.view = View::Month;
                    self.reset();
                }
                KeyCode::Char('c') => {
                    let id = named_id("habit/creatine");
                    ensure!(
                        self.today_habits().iter().any(|(i, _)| *i == id),
                        "Creatina no está programada para esta fecha"
                    );
                    self.choose(Choice::Habit(id, HabitState::Done), tx)?;
                }
                KeyCode::Enter => match index {
                    0 => self.weight_form(self.date),
                    1 => self.attendance_picker()?,
                    2 => {
                        if let Some(s) = self
                            .day(self.date)
                            .sessions
                            .iter()
                            .find(|s| s.status == SessionStatus::InProgress)
                        {
                            self.view = View::Session(self.date, s.id);
                            self.reset();
                        } else {
                            self.session_form(self.date, None);
                        }
                    }
                    i => {
                        if let Some((id, _)) = self.today_habits().get(i - 3) {
                            self.habit_picker(*id)?;
                        }
                    }
                },
                _ => (),
            },
            1 => match key {
                KeyCode::Char('n') | KeyCode::Char('w') => self.weight_form(self.date),
                KeyCode::Enter | KeyCode::Char('e') => {
                    if let Some(d) = self
                        .days()
                        .into_iter()
                        .filter(|d| d.weight.is_some())
                        .nth(index)
                    {
                        self.weight_form(d.date);
                    }
                }
                KeyCode::Char('d') => {
                    if let Some(d) = self
                        .days()
                        .into_iter()
                        .filter(|d| d.weight.is_some())
                        .nth(index)
                    {
                        let mut doc = self.day_doc(d.date);
                        if let Entity::Day(d) = &mut doc.entity {
                            d.weight = None;
                        }
                        self.confirm_save("¿Eliminar el peso seleccionado?", doc);
                    }
                }
                _ => (),
            },
            2 => match key {
                KeyCode::Char('n') => self.session_form(self.date, None),
                KeyCode::Char('r') => self.routine_picker()?,
                KeyCode::Enter => {
                    if let Some((date, s)) = self.sessions().get(index) {
                        self.date = *date;
                        self.view = View::Session(*date, s.id);
                        self.reset();
                    }
                }
                KeyCode::Char('e') => {
                    if let Some((date, s)) = self.sessions().get(index) {
                        self.session_form(*date, Some(s.id));
                    }
                }
                KeyCode::Char('d') => {
                    if let Some((date, s)) = self.sessions().get(index) {
                        let mut doc = self.day_doc(*date);
                        if let Entity::Day(d) = &mut doc.entity {
                            d.sessions.retain(|v| v.id != s.id);
                        }
                        self.confirm_save("¿Eliminar la sesión y todas sus series?", doc);
                    }
                }
                _ => (),
            },
            3 => match key {
                KeyCode::Char('n') => self.form(
                    "Nuevo ejercicio",
                    FormKind::Exercise,
                    vec![("Nombre", String::new())],
                    None,
                ),
                KeyCode::Enter | KeyCode::Char('e') => {
                    if let Some(doc) = self.docs("exercise").get(index) {
                        let Entity::Exercise(e) = &doc.entity else {
                            unreachable!()
                        };
                        self.form(
                            "Editar ejercicio",
                            FormKind::Exercise,
                            vec![("Nombre", e.name.clone())],
                            Some(doc.clone()),
                        );
                    }
                }
                KeyCode::Char('a') => {
                    if let Some(mut doc) = self.docs("exercise").get(index).cloned() {
                        if let Entity::Exercise(e) = &mut doc.entity {
                            e.archived = !e.archived;
                        }
                        self.confirm_save(
                            "¿Cambiar el archivo del ejercicio? El historial se conserva.",
                            doc,
                        );
                    }
                }
                _ => (),
            },
            4 => match key {
                KeyCode::Char('n') => self.form(
                    "Nueva rutina",
                    FormKind::Routine,
                    vec![("Nombre", String::new())],
                    None,
                ),
                KeyCode::Enter => {
                    if let Some(d) = self.docs("routine").get(index) {
                        self.view = View::Routine(d.id);
                        self.reset();
                    }
                }
                KeyCode::Char('e') => {
                    if let Some(d) = self.docs("routine").get(index) {
                        let Entity::Routine(r) = &d.entity else {
                            unreachable!()
                        };
                        self.form(
                            "Nombre de rutina",
                            FormKind::Routine,
                            vec![("Nombre", r.name.clone())],
                            Some(d.clone()),
                        );
                    }
                }
                KeyCode::Char('a') => {
                    if let Some(mut doc) = self.docs("routine").get(index).cloned() {
                        if let Entity::Routine(r) = &mut doc.entity {
                            r.archived = !r.archived;
                        }
                        self.confirm_save("¿Cambiar el archivo de la rutina?", doc);
                    }
                }
                KeyCode::Char('r') => {
                    if let Some(doc) = self.docs("routine").get(index) {
                        self.choose(Choice::StartRoutine(doc.id), tx)?;
                    }
                }
                _ => (),
            },
            5 => match key {
                KeyCode::Char('n') => self.form(
                    "Nuevo hábito sí/no",
                    FormKind::Habit,
                    vec![
                        ("Nombre", String::new()),
                        (
                            "Días (1=lun … 7=dom, separados por coma)",
                            "1,2,3,4,5,6,7".into(),
                        ),
                        ("Hora HH:MM (opcional)", String::new()),
                    ],
                    None,
                ),
                KeyCode::Enter | KeyCode::Char('e') => {
                    if let Some(doc) = self.docs("habit").get(index) {
                        let Entity::Habit(h) = &doc.entity else {
                            unreachable!()
                        };
                        self.form(
                            "Configurar hábito",
                            FormKind::Habit,
                            vec![
                                ("Nombre", h.name.clone()),
                                (
                                    "Días (1=lun … 7=dom)",
                                    h.weekdays
                                        .iter()
                                        .map(u8::to_string)
                                        .collect::<Vec<_>>()
                                        .join(","),
                                ),
                                (
                                    "Hora HH:MM (opcional)",
                                    h.reminder.clone().unwrap_or_default(),
                                ),
                            ],
                            Some(doc.clone()),
                        );
                    }
                }
                KeyCode::Char('t') => {
                    if let Some(d) = self.docs("habit").get(index) {
                        self.habit_picker(d.id)?;
                    }
                }
                KeyCode::Char('H') => {
                    if let Some(d) = self.docs("habit").get(index) {
                        self.view = View::HabitHistory(d.id);
                        self.reset();
                    }
                }
                KeyCode::Char('a') => {
                    if let Some(mut doc) = self.docs("habit").get(index).cloned() {
                        if let Entity::Habit(h) = &mut doc.entity {
                            h.archived = !h.archived;
                        }
                        self.confirm_save(
                            "¿Cambiar el archivo del hábito? El historial se conserva.",
                            doc,
                        );
                    }
                }
                _ => (),
            },
            6 => match key {
                KeyCode::Enter => {
                    if let Some(d) = self.docs("exercise").get(index) {
                        self.view = View::ExerciseHistory(d.id);
                        self.reset();
                    }
                }
                KeyCode::Char('m') => {
                    self.view = View::Month;
                    self.reset();
                }
                _ => (),
            },
            7 if key == KeyCode::Enter => {
                if let Some(value) = self.snapshot.conflicts.get(index) {
                    self.modal = Some(Modal::Conflict {
                        value: value.clone(),
                        scroll: 0,
                    });
                }
            }
            _ => (),
        }
        Ok(())
    }
    fn session_action(
        &mut self,
        key: KeyCode,
        date: NaiveDate,
        id: Uuid,
        tx: &mpsc::Sender<Command>,
    ) -> Result<()> {
        match key {
            KeyCode::Char('a') => self.exercise_picker(self.view)?,
            KeyCode::Char('r') => self.session_form(date, Some(id)),
            KeyCode::Char('n') => {
                let (i, _) = self
                    .training_row()
                    .context("Añade primero un ejercicio con a")?;
                self.set_form(date, id, i, None)?;
            }
            KeyCode::Enter | KeyCode::Char('e') => {
                let (i, j) = self.training_row().context("Añade un ejercicio con a")?;
                self.set_form(date, id, i, j)?;
            }
            KeyCode::Char('d') => {
                if let Some((i, j)) = self.training_row() {
                    let mut doc = self.day_doc(date);
                    if let Entity::Day(d) = &mut doc.entity {
                        let s = d
                            .sessions
                            .iter_mut()
                            .find(|s| s.id == id)
                            .context("Sesión inexistente")?;
                        if let Some(j) = j {
                            s.exercises[i].sets.remove(j);
                        } else {
                            s.exercises.remove(i);
                        }
                    }
                    self.confirm_save(
                        "¿Eliminar el ejercicio y sus series, o la serie seleccionada?",
                        doc,
                    );
                }
            }
            KeyCode::Char('c') => {
                let mut doc = self.day_doc(date);
                if let Entity::Day(d) = &mut doc.entity {
                    d.complete_session(id)?;
                }
                self.save(doc, tx)?;
            }
            KeyCode::Char('o') => {
                let mut doc = self.day_doc(date);
                if let Entity::Day(d) = &mut doc.entity {
                    let s = d
                        .sessions
                        .iter_mut()
                        .find(|s| s.id == id)
                        .context("Sesión inexistente")?;
                    s.status = SessionStatus::InProgress;
                }
                self.save(doc, tx)?;
            }
            _ => (),
        }
        Ok(())
    }
    fn routine_action(&mut self, key: KeyCode, id: Uuid, tx: &mpsc::Sender<Command>) -> Result<()> {
        let mut doc = self.document(id).context("Rutina inexistente")?;
        let original = doc.clone();
        let Entity::Routine(r) = &mut doc.entity else {
            unreachable!()
        };
        let index = self.index();
        match key {
            KeyCode::Char('a') => self.exercise_picker(self.view)?,
            KeyCode::Char('e') | KeyCode::Enter => {
                if let Some(e) = r.entries.get(index) {
                    self.form(
                        "Objetivos opcionales",
                        FormKind::Targets(index),
                        vec![
                            (
                                "Series (vacío: sin objetivo)",
                                e.target_sets.map_or(String::new(), |v| v.to_string()),
                            ),
                            (
                                "Repeticiones (opcional)",
                                e.target_reps.map_or(String::new(), |v| v.to_string()),
                            ),
                        ],
                        Some(original),
                    );
                }
            }
            KeyCode::Char('d') => {
                if index < r.entries.len() {
                    r.entries.remove(index);
                    self.confirm_save("¿Quitar el ejercicio de esta rutina?", doc);
                }
            }
            KeyCode::Char('K') => {
                if index > 0 && index < r.entries.len() {
                    r.entries.swap(index, index - 1);
                    self.save(doc, tx)?;
                    self.table.select(Some(index - 1));
                }
            }
            KeyCode::Char('J') => {
                if index + 1 < r.entries.len() {
                    r.entries.swap(index, index + 1);
                    self.save(doc, tx)?;
                    self.table.select(Some(index + 1));
                }
            }
            KeyCode::Char('r') => self.choose(Choice::StartRoutine(id), tx)?,
            _ => (),
        }
        Ok(())
    }
    fn rows(&self) -> Vec<Vec<String>> {
        let row = |a: String, b: String, c: String, d: String| vec![a, b, c, d];
        match self.view {
            View::Session(date, id) => {
                let mut rows = vec![];
                if let Some(s) = self.day(date).sessions.iter().find(|s| s.id == id) {
                    for (i, e) in s.exercises.iter().enumerate() {
                        rows.push(row(
                            format!("{}. {}", i + 1, e.name),
                            String::new(),
                            format!("Meta: {} × {}", opt(e.target_sets), opt(e.target_reps)),
                            "Enter: serie".into(),
                        ));
                        for (j, set) in e.sets.iter().enumerate() {
                            rows.push(row(
                                format!("    Serie {}", j + 1),
                                format!("{} kg", kg(set.load)),
                                format!("{} reps", set.reps),
                                String::new(),
                            ));
                        }
                    }
                }
                rows
            }
            View::Routine(id) => {
                if let Some(Document {
                    entity: Entity::Routine(r),
                    ..
                }) = self.document(id)
                {
                    r.entries
                        .iter()
                        .enumerate()
                        .map(|(i, e)| {
                            row(
                                format!("{}. {}", i + 1, self.exercise_name(e.exercise_id)),
                                opt(e.target_sets),
                                opt(e.target_reps),
                                String::new(),
                            )
                        })
                        .collect()
                } else {
                    vec![]
                }
            }
            View::Month => {
                let first = self.date.with_day(1).unwrap();
                let Some(end) = first.checked_add_months(Months::new(1)) else {
                    return vec![];
                };
                first
                    .iter_days()
                    .take_while(|d| *d < end)
                    .map(|date| {
                        let d = self.day(date);
                        row(
                            format!(
                                "{} {}",
                                date,
                                ["lun", "mar", "mié", "jue", "vie", "sáb", "dom"]
                                    [date.weekday().num_days_from_monday() as usize]
                            ),
                            d.attendance.label().into(),
                            d.weight.map_or("—".into(), |w| format!("{} kg", kg(w))),
                            format!("{} sesiones", d.sessions.len()),
                        )
                    })
                    .collect()
            }
            View::ExerciseHistory(id) => self
                .sessions()
                .into_iter()
                .flat_map(|(date, s)| {
                    s.exercises
                        .into_iter()
                        .filter(move |e| e.exercise_id == id)
                        .map(move |e| {
                            let sets = e
                                .sets
                                .iter()
                                .map(|s| format!("{} kg × {}", kg(s.load), s.reps))
                                .collect::<Vec<_>>()
                                .join(" · ");
                            row(
                                date.to_string(),
                                s.name.clone(),
                                sets,
                                status(s.status).into(),
                            )
                        })
                })
                .collect(),
            View::HabitHistory(id) => self
                .days()
                .into_iter()
                .filter_map(|d| {
                    d.habits.get(&id).map(|state| {
                        row(
                            d.date.to_string(),
                            state.label().into(),
                            String::new(),
                            String::new(),
                        )
                    })
                })
                .collect(),
            View::Main => match self.tab {
                0 => {
                    let day = self.day(self.date);
                    let completed = day
                        .sessions
                        .iter()
                        .filter(|s| s.status == SessionStatus::Completed)
                        .count();
                    let mut rows = vec![
                        row(
                            "Peso corporal".into(),
                            day.weight
                                .map_or("Pendiente".into(), |v| format!("{} kg", kg(v))),
                            "w / Enter".into(),
                            String::new(),
                        ),
                        row(
                            "Asistencia".into(),
                            day.attendance.label().into(),
                            "a / Enter".into(),
                            String::new(),
                        ),
                        row(
                            "Entrenamiento".into(),
                            if completed > 0 {
                                format!("{completed} completada(s)")
                            } else if day.sessions.is_empty() {
                                "Pendiente".into()
                            } else {
                                "En curso".into()
                            },
                            "n nueva · r rutina".into(),
                            String::new(),
                        ),
                    ];
                    let now = Utc::now().with_timezone(&self.timezone);
                    for (id, h) in self.today_habits() {
                        let state = day.habits.get(&id).copied().unwrap_or_default();
                        let reminder = h.reminder.as_ref().map_or(String::new(), |t| {
                            format!(
                                "{t}{}",
                                if state == HabitState::Pending
                                    && (self.date < now.date_naive()
                                        || self.date == now.date_naive()
                                            && now.format("%H:%M").to_string() >= *t)
                                {
                                    " · pendiente"
                                } else {
                                    ""
                                }
                            )
                        });
                        rows.push(row(
                            format!("{}{}", h.name, if h.archived { " [archivado]" } else { "" }),
                            state.label().into(),
                            reminder,
                            "Enter: estado".into(),
                        ));
                    }
                    rows
                }
                1 => self
                    .days()
                    .into_iter()
                    .filter_map(|d| {
                        d.weight.map(|w| {
                            row(
                                d.date.to_string(),
                                format!("{} kg", kg(w)),
                                String::new(),
                                String::new(),
                            )
                        })
                    })
                    .collect(),
                2 => self
                    .sessions()
                    .iter()
                    .map(|(date, s)| {
                        row(
                            date.to_string(),
                            s.name.clone(),
                            status(s.status).into(),
                            format!(
                                "{} series",
                                s.exercises.iter().map(|e| e.sets.len()).sum::<usize>()
                            ),
                        )
                    })
                    .collect(),
                3 => self
                    .docs("exercise")
                    .into_iter()
                    .map(|d| {
                        let Entity::Exercise(e) = d.entity else {
                            unreachable!()
                        };
                        row(
                            e.name,
                            archived(e.archived).into(),
                            String::new(),
                            String::new(),
                        )
                    })
                    .collect(),
                4 => self
                    .docs("routine")
                    .into_iter()
                    .map(|d| {
                        let Entity::Routine(r) = d.entity else {
                            unreachable!()
                        };
                        row(
                            r.name,
                            format!("{} ejercicios", r.entries.len()),
                            archived(r.archived).into(),
                            String::new(),
                        )
                    })
                    .collect(),
                5 => self
                    .docs("habit")
                    .into_iter()
                    .map(|d| {
                        let Entity::Habit(h) = d.entity else {
                            unreachable!()
                        };
                        row(
                            h.name,
                            h.weekdays
                                .iter()
                                .map(u8::to_string)
                                .collect::<Vec<_>>()
                                .join(","),
                            h.reminder.unwrap_or("Sin hora".into()),
                            archived(h.archived).into(),
                        )
                    })
                    .collect(),
                6 => self
                    .docs("exercise")
                    .into_iter()
                    .map(|doc| {
                        let sets: Vec<Set> = self
                            .sessions()
                            .into_iter()
                            .flat_map(|(_, s)| {
                                s.exercises
                                    .into_iter()
                                    .filter(|e| e.exercise_id == doc.id)
                                    .flat_map(|e| e.sets)
                            })
                            .collect();
                        let max = sets.iter().map(|s| s.load).max();
                        let reps = max.and_then(|v| {
                            sets.iter().filter(|s| s.load == v).map(|s| s.reps).max()
                        });
                        row(
                            self.exercise_name(doc.id),
                            max.map_or("Sin series".into(), |v| format!("{} kg", kg(v))),
                            reps.map_or("—".into(), |v| format!("{v} reps a esa carga")),
                            format!("{} series registradas", sets.len()),
                        )
                    })
                    .collect(),
                7 => self
                    .snapshot
                    .conflicts
                    .iter()
                    .map(|c| {
                        row(
                            c.local.name(),
                            "Revisión necesaria".into(),
                            c.current
                                .as_ref()
                                .map_or("Servidor: inexistente".into(), |r| {
                                    format!("Servidor: v{}", r.version)
                                }),
                            "Enter: comparar".into(),
                        )
                    })
                    .collect(),
                _ => vec![],
            },
        }
    }
    fn exercise_name(&self, id: Uuid) -> String {
        self.document(id)
            .and_then(|d| {
                if let Entity::Exercise(e) = d.entity {
                    Some(e.name)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "Ejercicio archivado".into())
    }
    fn title(&self) -> String {
        match self.view {
            View::Main => {
                if self.tab == 1 {
                    let weights: Vec<_> =
                        self.days().into_iter().filter_map(|d| d.weight).collect();
                    if weights.len() > 1 {
                        let diff = weights[0] as i64 - weights[weights.len() - 1] as i64;
                        format!(
                            "Peso · variación registrada: {}{} kg",
                            if diff >= 0 { "+" } else { "−" },
                            kg(diff.unsigned_abs() as u32)
                        )
                    } else {
                        "Peso corporal · historial en kg".into()
                    }
                } else {
                    TABS[self.tab].into()
                }
            }
            View::Session(date, id) => self
                .day(date)
                .sessions
                .iter()
                .find(|s| s.id == id)
                .map_or("Sesión".into(), |s| {
                    format!("{} · {} · {}", s.name, date, status(s.status))
                }),
            View::Routine(id) => self.document(id).map_or("Rutina".into(), |d| d.name()),
            View::Month => {
                let month = self.date.format("%Y-%m");
                let days: Vec<_> = self
                    .days()
                    .into_iter()
                    .filter(|d| {
                        d.date.year() == self.date.year() && d.date.month() == self.date.month()
                    })
                    .collect();
                let trained = days
                    .iter()
                    .filter(|d| d.attendance == Attendance::Trained)
                    .count();
                let rest = days
                    .iter()
                    .filter(|d| d.attendance == Attendance::Rest)
                    .count();
                let missed = days
                    .iter()
                    .filter(|d| d.attendance == Attendance::Missed)
                    .count();
                format!("{month} · Entrené {trained} · Descanso {rest} · Faltas {missed}")
            }
            View::ExerciseHistory(id) => format!(
                "{} · series reales, sin estimar 1RM",
                self.exercise_name(id)
            ),
            View::HabitHistory(id) => {
                self.document(id).map_or("Historial de hábito".into(), |d| {
                    format!("{} · historial", d.name())
                })
            }
        }
    }
    fn help_line(&self) -> &'static str {
        match self.view {
            View::Session(..)=>"a ejercicio · n serie · l/e editar · d eliminar · r notas · c completar · o reabrir · h volver",
            View::Routine(_)=>"a ejercicio · l/e objetivos · J/K ordenar · d quitar · r iniciar sesión · h volver",
            View::Month=>"[ / ] mes anterior/siguiente · l abrir día · D fecha · h volver",
            View::ExerciseHistory(_)|View::HabitHistory(_)=>"j/k mover · gg/G inicio/final · Ctrl+d/u media página · h volver",
            View::Main=>match self.tab {
                0=>"w peso · a asistencia · n sesión · r rutina · c creatina · m mes · l abrir",
                1=>"n peso en fecha seleccionada · l/e editar · d eliminar · D otra fecha",
                2=>"n nueva · r desde rutina · l continuar · e notas · d eliminar sesión",
                3=>"n nuevo · l/e editar · a archivar o restaurar",
                4=>"n nueva · l ejercicios · e nombre · a archivar · r iniciar sesión",
                5=>"n nuevo · l/e configurar · t estado del día · H historial · a archivar",
                6=>"l ver series del ejercicio · m asistencia mensual · 2 peso corporal",
                _=>"l comparar propuestas · L local / S servidor dentro de la comparación"
            }
        }
    }
    fn reference(&self) -> String {
        if let View::Session(date, id) = self.view {
            if let Some((index, _)) = self.training_row() {
                if let Some(s) = self.day(date).sessions.iter().find(|s| s.id == id) {
                    let e = &s.exercises[index];
                    if let Some((date, sets)) =
                        previous_exercise(&self.snapshot.documents, e.exercise_id, date, id)
                    {
                        return format!(
                            "Anterior {date}: {}",
                            sets.iter()
                                .map(|s| format!("{} kg × {}", kg(s.load), s.reps))
                                .collect::<Vec<_>>()
                                .join(" · ")
                        );
                    }
                    return "Sin sesión anterior registrada para este ejercicio".into();
                }
            }
        }
        String::new()
    }
    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        self.terminal_area = area;
        if area.width < 30 || area.height < 10 {
            frame.render_widget(
                Paragraph::new(
                    "LarpGym necesita 30 columnas y 10 filas. Amplía la terminal. q salir",
                )
                .wrap(Wrap { trim: false }),
                area,
            );
            return;
        }
        let pieces = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Length(2),
        ])
        .split(area);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    " LARPGYM ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(ACCENT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(" {} · {}", self.date, self.timezone)),
            ])),
            pieces[0],
        );
        let tabs: Line = if area.width >= 94 {
            TABS.iter()
                .enumerate()
                .map(|(i, name)| {
                    Span::styled(
                        format!(" {} {} ", i + 1, name),
                        if i == self.tab {
                            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        },
                    )
                })
                .collect()
        } else {
            Line::from(format!(
                " {} {} · gt/gT paneles · 1–8",
                self.tab + 1,
                TABS[self.tab]
            ))
        };
        frame.render_widget(Paragraph::new(tabs), pieces[1]);
        let mut content = pieces[2];
        if self.view == View::Main && self.tab == 1 && area.height >= 23 {
            let parts =
                Layout::vertical([Constraint::Min(3), Constraint::Length(4)]).split(content);
            content = parts[0];
            let weights: Vec<u64> = self
                .days()
                .into_iter()
                .rev()
                .filter_map(|d| d.weight.map(u64::from))
                .collect();
            if let Some(min) = weights.iter().min() {
                let data: Vec<u64> = weights.iter().map(|v| v - min).collect();
                frame.render_widget(
                    Sparkline::default()
                        .block(Block::bordered().title(format!(
                            "Tendencia · base {} kg · muestras por fecha",
                            kg(*min as u32)
                        )))
                        .data(&data)
                        .style(Style::default().fg(ACCENT)),
                    parts[1],
                );
            }
        }
        let rows = self.rows();
        self.visible_rows = (content.height.saturating_sub(2) as usize
            / if area.width < 85 { 3 } else { 1 })
        .max(1);
        if self.index() >= rows.len() {
            self.table.select(Some(rows.len().saturating_sub(1)));
        }
        let mut block = Block::bordered().title(format!(" {} ", self.title()));
        if !self.ready {
            block = block.title(" Cargando base local… ");
        }
        if rows.is_empty() {
            frame.render_widget(
                Paragraph::new(if self.ready {
                    "Sin registros. Usa los atajos de abajo para empezar."
                } else {
                    "Abriendo SQLite…"
                })
                .block(block)
                .wrap(Wrap { trim: false }),
                content,
            );
        } else {
            let narrow = area.width < 85;
            let display: Vec<Row> = rows
                .into_iter()
                .map(|r| {
                    if narrow {
                        Row::new(vec![
                            r[0].clone(),
                            r[1..]
                                .iter()
                                .filter(|s| !s.is_empty())
                                .cloned()
                                .collect::<Vec<_>>()
                                .join("\n"),
                        ])
                        .height(3)
                    } else {
                        Row::new(r)
                    }
                })
                .collect();
            let widths = if narrow {
                vec![Constraint::Percentage(43), Constraint::Percentage(57)]
            } else {
                vec![
                    Constraint::Percentage(30),
                    Constraint::Percentage(25),
                    Constraint::Percentage(25),
                    Constraint::Percentage(20),
                ]
            };
            let table = Table::new(display, widths)
                .block(block)
                .row_highlight_style(
                    Style::default()
                        .bg(Color::DarkGray)
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("› ");
            frame.render_stateful_widget(table, content, &mut self.table);
        }
        let detail = if self.message.is_empty()
            || self.message == "Guardado localmente" && !self.reference().is_empty()
        {
            self.reference()
        } else {
            self.message.clone()
        };
        frame.render_widget(
            Paragraph::new(format!("{}\n{}", detail, self.network)).style(Style::default().fg(
                if self.message.starts_with("No se guardó") {
                    Color::Yellow
                } else {
                    Color::White
                },
            )),
            pieces[3],
        );
        let sync = match &self.snapshot.last_sync {
            Some(s) => format!("Últ. sync: {}", s),
            None => "Últ. sync: nunca".into(),
        };
        frame.render_widget(
            Paragraph::new(format!(
                "Pendientes: {} · Conflictos: {} · {sync}",
                self.snapshot.pending,
                self.snapshot.conflicts.len()
            ))
            .style(Style::default().fg(ACCENT)),
            pieces[4],
        );
        frame.render_widget(
            Paragraph::new(format!("{}\n{}", self.help_line(), if self.pending_g {
                "g… pulsa g inicio · t panel siguiente · T anterior · Esc cancelar"
            } else {
                "h/j/k/l navegar · gg/G · Ctrl+d/u · gt/gT · D fecha · s sync · ? ayuda · q salir"
            })),
            pieces[5],
        );
        if let Some(modal) = &mut self.modal {
            draw_modal(frame, modal, &self.message);
        }
    }
}
fn optional_number(value: &str) -> Result<Option<u16>> {
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(
            value
                .parse()
                .context("El objetivo debe ser un entero positivo")?,
        ))
    }
}
fn opt(value: Option<u16>) -> String {
    value.map_or("—".into(), |v| v.to_string())
}
fn archived(value: bool) -> &'static str {
    if value {
        "Archivado"
    } else {
        "Activo"
    }
}
fn status(value: SessionStatus) -> &'static str {
    if value == SessionStatus::Completed {
        "Completada"
    } else {
        "En curso"
    }
}
fn popup(area: Rect, height: u16) -> Rect {
    let width = area.width.min(96);
    let height = area.height.min(height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}
fn draw_modal(frame: &mut Frame, modal: &mut Modal, message: &str) {
    let area = popup(
        frame.area(),
        match modal {
            Modal::Form(f) => (f.fields.len() * 3 + 7) as u16,
            Modal::Confirm { .. } => 10,
            _ => frame.area().height.saturating_sub(2),
        },
    );
    frame.render_widget(Clear, area);
    let title = match modal {
        Modal::Form(f) => f.title.as_str(),
        Modal::Picker { title, .. } => title,
        Modal::Confirm { .. } => "Confirmar · s sí / n no / Esc cancelar",
        Modal::Conflict { .. } => "Comparar · L local / S servidor · j/k · h cerrar",
        Modal::Help { .. } => "Ayuda · j/k · gg/G · Ctrl+d/u · h cerrar",
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT))
        .title(format!(" {title} "));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    match modal {
        Modal::Form(f) => {
            let parts = Layout::vertical([Constraint::Min(1), Constraint::Length(3)]).split(inner);
            let width = parts[0].width.saturating_sub(2) as usize;
            let mut lines = vec![];
            for (i, (label, value)) in f.fields.iter().enumerate() {
                let active = i == f.focus;
                lines.push(Line::styled(
                    format!("{} {label}", if active { "›" } else { " " }),
                    if active {
                        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                ));
                let cursor = if active {
                    f.cursor.min(value.len())
                } else {
                    value.len()
                };
                let before: Vec<_> = value[..cursor].chars().collect();
                let tail: String = before[before.len().saturating_sub(width.saturating_sub(3))..]
                    .iter()
                    .collect();
                let after: String = value[cursor..]
                    .chars()
                    .take(width.saturating_sub(tail.chars().count() + 1))
                    .collect();
                lines.push(Line::from(format!(
                    "  {tail}{}{after}",
                    if active { "▏" } else { "" }
                )));
                lines.push(Line::from(""));
            }
            let visible = parts[0].height as usize;
            let offset = (f.focus * 3 + 2).saturating_sub(visible);
            frame.render_widget(Paragraph::new(lines).scroll((offset as u16, 0)), parts[0]);
            frame.render_widget(Paragraph::new(format!("Tab/↑↓ campo · Enter siguiente/guardar · Ctrl+Enter guardar\nCtrl+U vaciar · Esc cancelar\n{message}")).style(Style::default().fg(Color::Yellow)),parts[1]);
        }
        Modal::Picker {
            items,
            selected,
            offset,
            ..
        } => {
            let parts = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);
            let list = List::new(items.iter().map(|(label, _)| ListItem::new(label.as_str())))
                .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White))
                .highlight_symbol("› ");
            let mut state = ListState::default()
                .with_selected(Some(*selected))
                .with_offset(*offset);
            frame.render_stateful_widget(list, parts[0], &mut state);
            *offset = state.offset();
            frame.render_widget(
                Paragraph::new("j/k · gg/G · l/Enter elegir · h/Esc volver"),
                parts[1],
            );
        }
        Modal::Confirm {
            message: prompt, ..
        } => frame.render_widget(
            Paragraph::new(format!(
                "{prompt}\n\nPulsa s para confirmar. n o Esc para cancelar.\n{message}"
            ))
            .wrap(Wrap { trim: false }),
            inner,
        ),
        Modal::Help { scroll } => {
            scroll_text(scroll, Motion::Ignore, HELP, inner);
            frame.render_widget(
                Paragraph::new(HELP)
                    .scroll((*scroll, 0))
                    .wrap(Wrap { trim: false }),
                inner,
            );
        }
        Modal::Conflict { value, scroll } => {
            let text = conflict_text(value);
            scroll_text(scroll, Motion::Ignore, &text, inner);
            frame.render_widget(
                Paragraph::new(text)
                    .scroll((*scroll, 0))
                    .wrap(Wrap { trim: false }),
                inner,
            );
        }
    }
}
fn conflict_text(value: &Conflict) -> String {
    let mut lines = vec![
        format!("{} · {}", value.local.name(), value.reason),
        "L: elegir local · S: elegir servidor (requieren confirmación)".into(),
        "LOCAL (tu propuesta completa):".into(),
    ];
    lines.extend(describe(&value.local));
    if let Some(r) = &value.current {
        lines.push(format!(
            "SERVIDOR · versión {} · {}:",
            r.version, r.updated_at
        ));
        lines.extend(describe(&r.document));
    } else {
        lines.push("SERVIDOR: sin registro".into());
    }
    lines.push("La elección afecta al registro completo; un día incluye peso, hábitos y todas sus sesiones.".into());
    lines.push("Ambas propuestas se conservan en la auditoría SQLite local. La resolución se valida de nuevo al sincronizar.".into());
    lines.join("\n")
}
fn describe(doc: &Document) -> Vec<String> {
    let mut lines = vec![format!(
        "{}{}",
        doc.name(),
        if doc.deleted { " [ELIMINADO]" } else { "" }
    )];
    match &doc.entity {
        Entity::Exercise(e) => lines.push(archived(e.archived).into()),
        Entity::Routine(r) => {
            lines.push(archived(r.archived).into());
            for e in &r.entries {
                lines.push(format!(
                    "  {} · meta {}×{}",
                    e.exercise_id,
                    opt(e.target_sets),
                    opt(e.target_reps)
                ));
            }
        }
        Entity::Habit(h) => {
            lines.push(format!(
                "{} · días {:?} · hora {}",
                archived(h.archived),
                h.weekdays,
                h.reminder.as_deref().unwrap_or("sin hora")
            ));
        }
        Entity::Day(d) => {
            lines.push(format!(
                "Peso: {} · {}",
                d.weight
                    .map_or("sin registro".into(), |w| format!("{} kg", kg(w))),
                d.attendance.label()
            ));
            for (id, state) in &d.habits {
                lines.push(format!("Hábito {id}: {}", state.label()));
            }
            for s in &d.sessions {
                lines.push(format!(
                    "Sesión {} · {} · {}",
                    s.name,
                    status(s.status),
                    s.notes
                ));
                for e in &s.exercises {
                    lines.push(format!(
                        "  {}: {}",
                        e.name,
                        e.sets
                            .iter()
                            .map(|s| format!("{} kg × {}", kg(s.load), s.reps))
                            .collect::<Vec<_>>()
                            .join(" / ")
                    ));
                }
            }
        }
    }
    lines
}
const HELP: &str = r#"LARPGYM · Navegación tipo Vim

j/k o ↑↓: bajar/subir en listas y selectores.
h o ←: volver; l, → o Enter: abrir/elegir.
gg / G: inicio/final. g espera la segunda tecla.
Ctrl+d/u: media página; Ctrl+f/b: página completa.
gt / gT: panel siguiente/anterior (también Tab/Shift+Tab).
1–8: Hoy, Peso, Sesiones, Ejercicios, Rutinas, Hábitos,
Progreso y Conflictos. Los números eligen paneles.
[ / ]: cambiar fecha; D: escribir AAAA-MM-DD.
s: sincronizar; q o Ctrl+C: salir. ?: esta ayuda.
En selectores y ayuda, h, q o Esc cierran el diálogo.
Home/End y PgUp/PgDn también están disponibles.

HOY
w peso · a asistencia · n sesión vacía · r desde rutina.
c marca creatina completada; l cambia el estado de un hábito.
m abre el resumen mensual; [ / ] cambian mes allí.
Sin datos significa Sin registrar, nunca una falta automática.

ENTRENAMIENTO
a añade ejercicio; n añade serie al ejercicio seleccionado.
l/Enter/e edita serie; r cambia nombre y notas.
d elimina serie o ejercicio, previa confirmación.
c completa sesión y marca Entrené en la misma escritura.
o reabre una sesión para corregirla. Abrirla no la completa.
Los objetivos son referencias; no crean series realizadas.
Las sesiones en curso se recuperan al reiniciar.

CATÁLOGOS
n crea; e edita; a archiva/restaura con confirmación.
Rutina: l abre ejercicios; a añade, e objetivos, J/K ordenan.
Cambiar la rutina no altera sesiones anteriores.
Hábitos: t registra estado del día; H muestra historial.
Días: 1=lunes, 7=domingo. Hora opcional HH:MM.
Los recordatorios solo aparecen mientras esta TUI está abierta.

FORMULARIOS · ENTRADA DE TEXTO
Las letras h/j/k/l, g y G escriben texto en los campos.
Tab/↑↓ cambian campo; Enter avanza y al final guarda.
Ctrl+Enter guarda desde cualquier campo. Ctrl+U vacía.
←/→, Home/End, Backspace y Delete editan el campo.
Esc cancela; los errores conservan lo escrito.
Cada confirmación se guarda en SQLite junto con su operación.

SINCRONIZACIÓN Y CONFLICTOS
Automática al inicio y periódica; s permite solicitarla.
Sin servidor puedes seguir trabajando. La cola sobrevive al reinicio.
8 muestra conflictos; l abre la comparación.
Dentro de la comparación: L elige local y S elige servidor.
La elección requiere confirmar con s; n, h o Esc cancelan.
La elección afecta al día completo si el conflicto es de fecha.
El servidor comprueba otra vez la versión al recibir una resolución.
Los récords son series registradas; no hay estimación de 1RM.
"#;

#[cfg(test)]
mod navigation_tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    struct Harness {
        app: App,
        tx: mpsc::Sender<Command>,
        rx: mpsc::Receiver<Command>,
        sync: tokio::sync::mpsc::Sender<()>,
    }
    impl Harness {
        fn new() -> Self {
            let mut app = App::new(
                NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
                chrono_tz::America::Mazatlan,
            );
            app.ready = true;
            app.snapshot.documents = initial_catalogue();
            let (tx, rx) = mpsc::channel();
            let (sync, _) = tokio::sync::mpsc::channel(1);
            Self { app, tx, rx, sync }
        }
        fn event(&mut self, key: KeyEvent) {
            assert!(!self.app.key(key, &self.tx, &self.sync).unwrap());
        }
        fn press(&mut self, text: &str) {
            for c in text.chars() {
                self.event(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
            }
        }
        fn ctrl(&mut self, c: char) {
            self.event(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
        }
        fn escape(&mut self) {
            self.event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        }
        fn render(&mut self, width: u16, height: u16) -> String {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| self.app.draw(frame)).unwrap();
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect()
        }
    }

    #[test]
    fn vim_list_and_cancelled_prefix_never_trigger_delete() {
        let mut h = Harness::new();
        h.press("4jjk");
        assert_eq!(h.app.index(), 1);
        h.press("G");
        assert_eq!(h.app.index(), 5);
        h.press("g");
        assert!(h.app.pending_g);
        assert_eq!(h.app.index(), 5);
        h.press("g");
        assert_eq!(h.app.index(), 0);
        assert!(h.app.modal.is_none());
        h.press("gd");
        assert!(!h.app.pending_g);
        assert!(h.app.modal.is_none());
        h.ctrl('d');
        assert!(h.app.modal.is_none());
        assert!(h.rx.try_recv().is_err());
    }

    #[test]
    fn vim_tabs_and_text_entry_have_separate_keymaps() {
        let mut h = Harness::new();
        h.press("gt");
        assert_eq!(h.app.tab, 1);
        h.press("gT");
        assert_eq!(h.app.tab, 0);
        h.press("gT");
        assert_eq!(h.app.tab, 7);
        h.press("4l");
        h.press("hjklggGt");
        let Some(Modal::Form(form)) = &h.app.modal else {
            panic!("Expected text form")
        };
        assert!(form.fields[0].1.ends_with("hjklggGt"));
        assert!(!h.app.pending_g);
        h.escape();
        h.press("D");
        assert!(matches!(
            &h.app.modal,
            Some(Modal::Form(Form {
                kind: FormKind::Date,
                ..
            }))
        ));
        assert!(h.rx.try_recv().is_err());
    }

    #[test]
    fn picker_supports_vim_navigation_and_open() {
        let mut h = Harness::new();
        h.press("aGggjl");
        let Command::Save { document, .. } = h.rx.try_recv().unwrap() else {
            panic!("Expected save")
        };
        let Entity::Day(day) = document.entity else {
            panic!("Expected day")
        };
        assert_eq!(day.attendance, Attendance::Rest);
        assert!(h.app.saving);
    }

    #[test]
    fn half_pages_follow_terminal_height_and_row_height() {
        let mut h = Harness::new();
        for i in 0..40 {
            h.app.snapshot.documents.push(Document {
                id: Uuid::new_v4(),
                deleted: false,
                entity: Entity::Exercise(Exercise {
                    name: format!("Ejercicio {i}"),
                    archived: false,
                }),
            });
        }
        h.press("4");
        h.render(110, 30);
        h.ctrl('d');
        assert!(h.app.index() > 1 && h.app.index() < 45);
        h.ctrl('u');
        assert_eq!(h.app.index(), 0);
        h.render(40, 12);
        h.ctrl('d');
        assert_eq!(h.app.index(), 1);
        h.ctrl('f');
        assert_eq!(h.app.index(), 2);
        h.ctrl('b');
        assert_eq!(h.app.index(), 1);
        assert!(h.rx.try_recv().is_err());
    }

    #[test]
    fn help_scrolls_to_real_bottom_and_clamps_after_resize() {
        let mut h = Harness::new();
        h.press("?");
        h.render(40, 12);
        h.ctrl('d');
        assert!(matches!(h.app.modal,Some(Modal::Help { scroll }) if scroll>0));
        h.ctrl('u');
        assert!(matches!(h.app.modal, Some(Modal::Help { scroll: 0 })));
        h.press("G");
        let screen = h.render(40, 12);
        assert!(screen.contains("1RM"));
        h.press("gg");
        assert!(matches!(h.app.modal, Some(Modal::Help { scroll: 0 })));
        h.press("G");
        h.render(110, 90);
        assert!(matches!(h.app.modal, Some(Modal::Help { scroll: 0 })));
        h.press("h");
        assert!(h.app.modal.is_none());
    }

    #[test]
    fn conflict_open_motion_cannot_choose_local_proposal() {
        let mut h = Harness::new();
        let document = initial_catalogue().remove(0);
        h.app.modal = Some(Modal::Conflict {
            value: Conflict {
                entity_id: document.id,
                local: document,
                current: None,
                reason: "Concurrente".into(),
            },
            scroll: 0,
        });
        h.press("l");
        assert!(matches!(h.app.modal, Some(Modal::Conflict { .. })));
        h.press("L");
        assert!(matches!(
            h.app.modal,
            Some(Modal::Confirm {
                action: Action::Resolve(_, true),
                ..
            })
        ));
        h.press("h");
        assert!(h.app.modal.is_none());
        assert!(h.rx.try_recv().is_err());
        assert!(!h.app.saving);
    }

    #[test]
    fn h_returns_from_details_and_uppercase_h_opens_habit_history() {
        let mut h = Harness::new();
        let id = Uuid::new_v4();
        h.app.snapshot.documents.push(Document {
            id,
            deleted: false,
            entity: Entity::Routine(Routine {
                name: "Rutina".into(),
                entries: vec![],
                archived: false,
            }),
        });
        h.press("5l");
        assert!(matches!(h.app.view,View::Routine(rid) if rid==id));
        h.press("h");
        assert!(h.app.view == View::Main);
        h.press("6H");
        assert!(matches!(h.app.view, View::HabitHistory(_)));
        h.press("h");
        assert!(h.app.view == View::Main);
        assert!(h.rx.try_recv().is_err());
    }
}
