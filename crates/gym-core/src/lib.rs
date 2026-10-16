//! Portable domain and wire types. Dates are local; revision instants are UTC.
use anyhow::{ensure, Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use uuid::Uuid;

pub const MAX_BATCH: usize = 100;
pub const PAGE_SIZE: usize = 100;
pub const MAX_DOCUMENT_BYTES: usize = 128 * 1024;

/// Integer grams: exact to 0.001 kg. No floating point enters persistence.
pub type Grams = u32;
pub fn parse_kg(input: &str) -> Result<Grams> {
    let input = input.trim().replace(',', ".");
    let parts: Vec<_> = input.split('.').collect();
    ensure!(
        parts.len() <= 2 && !parts[0].is_empty(),
        "Peso inválido; usa kg, por ejemplo 72.500"
    );
    ensure!(
        parts.iter().all(|s| s.chars().all(|c| c.is_ascii_digit())),
        "Usa un peso positivo o cero en kg"
    );
    let whole: u32 = parts[0].parse().context("Peso fuera de rango")?;
    let fraction = parts.get(1).copied().unwrap_or("");
    ensure!(fraction.len() <= 3, "Máximo tres decimales en kg");
    let grams = whole
        .checked_mul(1000)
        .and_then(|v| v.checked_add(format!("{fraction:0<3}").parse::<u32>().ok()?))
        .ok_or_else(|| anyhow::anyhow!("Peso fuera de rango"))?;
    ensure!(grams <= 2_000_000, "El peso debe estar entre 0 y 2000 kg");
    Ok(grams)
}
pub fn kg(grams: Grams) -> String {
    format!("{}.{:03}", grams / 1000, grams % 1000)
}
pub fn named_id(name: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("larpgym/v1/{name}").as_bytes(),
    )
}
pub fn day_id(date: NaiveDate) -> Uuid {
    named_id(&format!("day/{date}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Attendance {
    Trained,
    Rest,
    Missed,
    #[default]
    Unrecorded,
}
impl Attendance {
    pub fn label(self) -> &'static str {
        match self {
            Self::Trained => "Entrené",
            Self::Rest => "Descanso programado",
            Self::Missed => "No asistí",
            Self::Unrecorded => "Sin registrar",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HabitState {
    Done,
    Skipped,
    #[default]
    Pending,
}
impl HabitState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Done => "Completado",
            Self::Skipped => "Omitido",
            Self::Pending => "Pendiente",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    InProgress,
    Completed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exercise {
    pub name: String,
    pub archived: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutineEntry {
    pub exercise_id: Uuid,
    pub target_sets: Option<u16>,
    pub target_reps: Option<u16>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Routine {
    pub name: String,
    pub entries: Vec<RoutineEntry>,
    pub archived: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Habit {
    pub name: String,
    /// ISO weekdays: Monday=1, Sunday=7.
    pub weekdays: Vec<u8>,
    pub reminder: Option<String>,
    pub archived: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Set {
    pub id: Uuid,
    pub load: Grams,
    pub reps: u16,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionExercise {
    pub id: Uuid,
    pub exercise_id: Uuid,
    /// Snapshot, independent of subsequent catalogue edits.
    pub name: String,
    pub target_sets: Option<u16>,
    pub target_reps: Option<u16>,
    pub sets: Vec<Set>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub id: Uuid,
    pub name: String,
    pub notes: String,
    pub status: SessionStatus,
    pub exercises: Vec<SessionExercise>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Day {
    pub date: NaiveDate,
    pub weight: Option<Grams>,
    pub attendance: Attendance,
    pub habits: BTreeMap<Uuid, HabitState>,
    pub sessions: Vec<Session>,
}
impl Day {
    pub fn new(date: NaiveDate) -> Self {
        Self {
            date,
            weight: None,
            attendance: Attendance::Unrecorded,
            habits: BTreeMap::new(),
            sessions: vec![],
        }
    }
    pub fn complete_session(&mut self, id: Uuid) -> Result<()> {
        let session = self
            .sessions
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or_else(|| anyhow::anyhow!("Sesión inexistente"))?;
        ensure!(
            session.exercises.iter().any(|e| !e.sets.is_empty()),
            "Registra al menos una serie antes de completar la sesión"
        );
        session.status = SessionStatus::Completed;
        self.attendance = Attendance::Trained;
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Entity {
    Exercise(Exercise),
    Routine(Routine),
    Habit(Habit),
    Day(Day),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub id: Uuid,
    pub deleted: bool,
    pub entity: Entity,
}
impl Document {
    pub fn day(date: NaiveDate) -> Self {
        Self {
            id: day_id(date),
            deleted: false,
            entity: Entity::Day(Day::new(date)),
        }
    }
    pub fn name(&self) -> String {
        match &self.entity {
            Entity::Day(d) => format!("Día {}", d.date),
            Entity::Exercise(e) => format!("Ejercicio: {}", e.name),
            Entity::Routine(r) => format!("Rutina: {}", r.name),
            Entity::Habit(h) => format!("Hábito: {}", h.name),
        }
    }
    pub fn kind(&self) -> &'static str {
        match self.entity {
            Entity::Day(_) => "day",
            Entity::Exercise(_) => "exercise",
            Entity::Routine(_) => "routine",
            Entity::Habit(_) => "habit",
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(!self.id.is_nil(), "ID vacío");
        ensure!(
            serde_json::to_vec(self)?.len() <= MAX_DOCUMENT_BYTES,
            "Registro demasiado grande (128 KiB)"
        );
        match &self.entity {
            Entity::Exercise(e) => name(&e.name)?,
            Entity::Routine(r) => {
                name(&r.name)?;
                ensure!(r.entries.len() <= 100, "Máximo 100 ejercicios");
                for e in &r.entries {
                    targets(e.target_sets, e.target_reps)?;
                }
            }
            Entity::Habit(h) => {
                name(&h.name)?;
                ensure!(
                    !h.weekdays.is_empty()
                        && h.weekdays.len() <= 7
                        && h.weekdays.iter().all(|d| (1..=7).contains(d))
                        && h.weekdays.iter().collect::<HashSet<_>>().len() == h.weekdays.len(),
                    "Días: números únicos entre 1 (lunes) y 7 (domingo)"
                );
                if let Some(t) = &h.reminder {
                    ensure!(
                        t.len() == 5
                            && t.as_bytes()[2] == b':'
                            && chrono::NaiveTime::parse_from_str(t, "%H:%M").is_ok(),
                        "Hora inválida; usa HH:MM"
                    );
                }
            }
            Entity::Day(d) => {
                ensure!(
                    self.id == day_id(d.date),
                    "El ID diario debe derivarse de su fecha"
                );
                if let Some(w) = d.weight {
                    ensure!((1..=2_000_000).contains(&w), "Peso corporal fuera de rango");
                }
                ensure!(
                    d.sessions.len() <= 30 && d.habits.len() <= 200,
                    "Demasiados registros en un día"
                );
                let mut ids = HashSet::new();
                for s in &d.sessions {
                    ensure!(!s.id.is_nil() && ids.insert(s.id), "ID de sesión duplicado");
                    name(&s.name)?;
                    ensure!(
                        s.notes.len() <= 4000
                            && !s.notes.chars().any(|c| c.is_control() && c != '\n')
                            && s.exercises.len() <= 100,
                        "Notas o sesión demasiado grandes"
                    );
                    if s.status == SessionStatus::Completed {
                        ensure!(
                            d.attendance == Attendance::Trained,
                            "Un día con sesiones completadas debe tener asistencia Entrené"
                        );
                        ensure!(
                            s.exercises.iter().any(|e| !e.sets.is_empty()),
                            "Una sesión completada necesita series"
                        );
                    }
                    for e in &s.exercises {
                        ensure!(
                            !e.id.is_nil() && ids.insert(e.id),
                            "ID de ejercicio realizado duplicado"
                        );
                        name(&e.name)?;
                        targets(e.target_sets, e.target_reps)?;
                        ensure!(e.sets.len() <= 200, "Máximo 200 series por ejercicio");
                        for set in &e.sets {
                            ensure!(
                                !set.id.is_nil() && ids.insert(set.id),
                                "ID de serie duplicado"
                            );
                            ensure!(
                                set.load <= 2_000_000 && (1..=1000).contains(&set.reps),
                                "Serie: 0–2000 kg y 1–1000 repeticiones"
                            );
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
fn name(s: &str) -> Result<()> {
    ensure!(
        !s.trim().is_empty() && s.len() <= 160 && !s.chars().any(char::is_control),
        "Nombre obligatorio, sin controles y de hasta 160 bytes"
    );
    Ok(())
}
fn targets(sets: Option<u16>, reps: Option<u16>) -> Result<()> {
    ensure!(
        sets.is_none_or(|v| (1..=200).contains(&v)) && reps.is_none_or(|v| (1..=1000).contains(&v)),
        "Objetivos fuera de rango"
    );
    Ok(())
}

/// Validate the whole candidate graph inside the same storage transaction as a write.
pub fn validate_graph(docs: &[Document]) -> Result<()> {
    let active: HashMap<_, _> = docs
        .iter()
        .filter(|d| !d.deleted)
        .map(|d| (d.id, d))
        .collect();
    let mut session_ids = HashSet::new();
    for doc in docs.iter().filter(|d| !d.deleted) {
        doc.validate()?;
        let exercise = |id: Uuid| -> Result<()> {
            ensure!(
                matches!(
                    active.get(&id).map(|d| &d.entity),
                    Some(Entity::Exercise(_))
                ),
                "Ejercicio referenciado inexistente; sincroniza o resuelve primero el catálogo"
            );
            Ok(())
        };
        match &doc.entity {
            Entity::Routine(r) => {
                for e in &r.entries {
                    exercise(e.exercise_id)?;
                }
            }
            Entity::Day(d) => {
                for id in d.habits.keys() {
                    ensure!(
                        matches!(active.get(id).map(|d| &d.entity), Some(Entity::Habit(_))),
                        "Hábito referenciado inexistente"
                    );
                }
                for s in &d.sessions {
                    ensure!(session_ids.insert(s.id), "Sesión duplicada entre fechas");
                    for e in &s.exercises {
                        exercise(e.exercise_id)?;
                    }
                }
            }
            _ => (),
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revision {
    pub document: Document,
    pub version: u64,
    pub updated_at: DateTime<Utc>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    pub id: Uuid,
    pub device_id: Uuid,
    pub base_version: u64,
    #[serde(default)]
    pub strict_version: bool,
    pub document: Document,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Outcome {
    Accepted {
        revision: Revision,
    },
    Conflict {
        current: Option<Revision>,
        reason: String,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PushRequest {
    pub operations: Vec<Operation>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushResponse {
    pub outcomes: Vec<Outcome>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Change {
    pub cursor: u64,
    pub revision: Revision,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page {
    pub changes: Vec<Change>,
    pub next_cursor: u64,
    pub has_more: bool,
}

pub fn initial_catalogue() -> Vec<Document> {
    let mut docs: Vec<_> = [
        "Sentadilla",
        "Press de banca",
        "Peso muerto",
        "Remo",
        "Press militar",
        "Dominadas",
    ]
    .into_iter()
    .map(|n| Document {
        id: named_id(&format!("exercise/{n}")),
        deleted: false,
        entity: Entity::Exercise(Exercise {
            name: n.into(),
            archived: false,
        }),
    })
    .collect();
    docs.push(Document {
        id: named_id("habit/creatine"),
        deleted: false,
        entity: Entity::Habit(Habit {
            name: "Creatina".into(),
            weekdays: vec![1, 2, 3, 4, 5, 6, 7],
            reminder: None,
            archived: false,
        }),
    });
    docs
}

/// Most recent session preceding this one (date, then insertion order within the day).
pub fn previous_exercise(
    docs: &[Document],
    exercise_id: Uuid,
    date: NaiveDate,
    session_id: Uuid,
) -> Option<(NaiveDate, Vec<Set>)> {
    let mut previous: Option<(NaiveDate, usize, Vec<Set>)> = None;
    for document in docs.iter().filter(|d| !d.deleted) {
        let Entity::Day(day) = &document.entity else {
            continue;
        };
        if day.date > date {
            continue;
        }
        let stop = if day.date == date {
            day.sessions
                .iter()
                .position(|s| s.id == session_id)
                .unwrap_or(day.sessions.len())
        } else {
            day.sessions.len()
        };
        for (i, session) in day.sessions.iter().take(stop).enumerate() {
            if let Some(exercise) = session
                .exercises
                .iter()
                .rev()
                .find(|e| e.exercise_id == exercise_id && !e.sets.is_empty())
            {
                if previous
                    .as_ref()
                    .is_none_or(|(d, j, _)| (day.date, i) > (*d, *j))
                {
                    previous = Some((day.date, i, exercise.sets.clone()));
                }
            }
        }
    }
    previous.map(|(d, _, sets)| (d, sets))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_weights() {
        assert_eq!(parse_kg("72,125").unwrap(), 72125);
        assert_eq!(parse_kg("0").unwrap(), 0);
        for s in ["-1", "NaN", "1.1234", ".1", "2001"] {
            assert!(parse_kg(s).is_err(), "{s}");
        }
    }
    #[test]
    fn completion_requires_real_sets() {
        let mut d = Day::new(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());
        let id = Uuid::new_v4();
        d.sessions.push(Session {
            id,
            name: "Fuerza".into(),
            notes: String::new(),
            status: SessionStatus::InProgress,
            exercises: vec![],
        });
        assert!(d.complete_session(id).is_err());
        assert_eq!(d.attendance, Attendance::Unrecorded);
    }
}
