//! 감시 경로 설정과 규칙 평가.
//!
//! 규칙 모델은 원본 nfd2nfc와 호환된다.
//! - `action = "watch" | "ignore"`
//! - `mode = "recursive" | "children"`
//! - ignore 규칙은 항상 recursive로 동작한다.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fs_util::{expand_tilde, nfc_path};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    #[default]
    Watch,
    Ignore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// 폴더와 모든 하위 폴더
    #[default]
    Recursive,
    /// 폴더 바로 아래 항목만
    Children,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathEntry {
    pub path: String,
    #[serde(default)]
    pub action: Action,
    #[serde(default)]
    pub mode: Mode,
}

impl PathEntry {
    /// ignore는 항상 recursive
    pub fn effective_mode(&self) -> Mode {
        match self.action {
            Action::Ignore => Mode::Recursive,
            Action::Watch => self.mode,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// 앱 실행 시 감시를 자동으로 시작할지 여부 (마지막 상태를 기억)
    pub watcher_enabled: bool,
    pub paths: Vec<PathEntry>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            watcher_enabled: true,
            paths: Vec::new(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("설정 파일을 읽을 수 없습니다: {0}")]
    Io(#[from] io::Error),
    #[error("설정 파일 형식이 잘못되었습니다: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("설정을 저장할 수 없습니다: {0}")]
    Serialize(#[from] toml::ser::Error),
}

impl Config {
    /// 파일이 없으면 기본 설정을 돌려준다.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        match fs::read_to_string(path) {
            Ok(s) => Ok(toml::from_str(&s)?),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// 임시 파일에 쓴 뒤 rename — 저장 도중 앱이 죽어도 설정이 반쯤 깨지지 않는다.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let body = toml::to_string_pretty(self)?;
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, body)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StatusKind {
    Active,
    NotFound,
    NotADirectory,
    PermissionDenied,
    /// 다른 규칙이 이미 같은 동작으로 덮고 있어 적용되지 않음
    Redundant,
    /// 같은 경로의 우선 규칙에 의해 무시됨
    Overridden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryStatus {
    pub kind: StatusKind,
    /// Active: 덮어쓰는 상위 규칙 / Redundant·Overridden: 원인이 된 규칙 (0-based)
    pub reference: Option<usize>,
}

impl EntryStatus {
    fn new(kind: StatusKind, reference: Option<usize>) -> Self {
        Self { kind, reference }
    }
}

/// 실제로 적용되는 규칙
#[derive(Debug, Clone)]
pub struct ActiveEntry {
    pub index: usize,
    /// 실제 경로 (canonicalize 결과)
    pub canonical: PathBuf,
    /// 비교용 NFC 키
    pub key: PathBuf,
    pub action: Action,
    pub mode: Mode,
}

fn resolve(raw: &str) -> Result<PathBuf, StatusKind> {
    let p = expand_tilde(raw);
    let meta = fs::metadata(&p).map_err(|e| match e.kind() {
        io::ErrorKind::PermissionDenied => StatusKind::PermissionDenied,
        _ => StatusKind::NotFound,
    })?;
    if !meta.is_dir() {
        return Err(StatusKind::NotADirectory);
    }
    if let Err(e) = fs::read_dir(&p) {
        if e.kind() == io::ErrorKind::PermissionDenied {
            return Err(StatusKind::PermissionDenied);
        }
    }
    fs::canonicalize(&p).map_err(|_| StatusKind::NotFound)
}

/// 각 규칙의 상태와, 실제로 적용될 규칙 목록을 계산한다.
///
/// 1단계(같은 경로): 먼저 적힌 규칙의 action이 이긴다. 같은 action끼리는 recursive가,
///   그다음 앞선 규칙이 이긴다. 나머지는 Redundant 또는 Overridden.
/// 2단계(부모·자식): 짧은 경로부터 처리한다. 가장 가까운 활성 recursive 조상과 action이
///   같으면 Redundant다. 다르면 조상을 덮어쓰는 Active가 된다.
///   예: 감시 중인 폴더 안의 하위 폴더만 제외하기.
pub fn evaluate(entries: &[PathEntry]) -> (Vec<EntryStatus>, Vec<ActiveEntry>) {
    let n = entries.len();
    let mut statuses: Vec<Option<EntryStatus>> = vec![None; n];
    let mut canon: Vec<Option<(PathBuf, PathBuf)>> = vec![None; n];

    for (i, e) in entries.iter().enumerate() {
        match resolve(&e.path) {
            Ok(p) => {
                let key = nfc_path(&p);
                canon[i] = Some((p, key));
            }
            Err(kind) => statuses[i] = Some(EntryStatus::new(kind, None)),
        }
    }

    // ── 1단계: 같은 경로 그룹 ──
    let mut order: Vec<PathBuf> = Vec::new();
    let mut groups: HashMap<PathBuf, Vec<usize>> = HashMap::new();
    for (i, c) in canon.iter().enumerate() {
        if let Some((_, key)) = c {
            groups
                .entry(key.clone())
                .or_insert_with(|| {
                    order.push(key.clone());
                    Vec::new()
                })
                .push(i);
        }
    }
    let mut winners = Vec::new();
    for key in &order {
        let idxs = &groups[key];
        let first_action = entries[idxs[0]].action;
        let winner = *idxs
            .iter()
            .filter(|&&i| entries[i].action == first_action)
            .min_by_key(|&&i| (entries[i].effective_mode() != Mode::Recursive, i))
            .expect("group is never empty");
        for &i in idxs {
            if i == winner {
                continue;
            }
            let kind = if entries[i].action == first_action {
                StatusKind::Redundant
            } else {
                StatusKind::Overridden
            };
            statuses[i] = Some(EntryStatus::new(kind, Some(winner)));
        }
        winners.push(winner);
    }

    // ── 2단계: 부모·자식 관계 ──
    let key_of = |i: usize| &canon[i].as_ref().expect("resolved").1;
    winners.sort_by_key(|&i| (key_of(i).components().count(), i));
    let mut active: Vec<usize> = Vec::new();
    for w in winners {
        let ancestor = active
            .iter()
            .copied()
            .filter(|&a| {
                entries[a].effective_mode() == Mode::Recursive
                    && key_of(w) != key_of(a)
                    && key_of(w).starts_with(key_of(a))
            })
            .max_by_key(|&a| key_of(a).components().count());
        let status = match ancestor {
            Some(a) if entries[a].action == entries[w].action => {
                EntryStatus::new(StatusKind::Redundant, Some(a))
            }
            other => {
                active.push(w);
                EntryStatus::new(StatusKind::Active, other)
            }
        };
        statuses[w] = Some(status);
    }

    active.sort_unstable();
    let actives = active
        .into_iter()
        .map(|i| {
            let (canonical, key) = canon[i].clone().expect("resolved");
            ActiveEntry {
                index: i,
                canonical,
                key,
                action: entries[i].action,
                mode: entries[i].effective_mode(),
            }
        })
        .collect();

    (
        statuses.into_iter().map(|s| s.expect("all set")).collect(),
        actives,
    )
}

/// 이벤트 경로(NFC 키)에 적용될 action. 가장 구체적인(긴) 규칙이 이긴다.
pub fn effective_action(key: &Path, active: &[ActiveEntry]) -> Option<Action> {
    active
        .iter()
        .filter(|e| match e.mode {
            Mode::Recursive => key.starts_with(&e.key),
            Mode::Children => key.parent() == Some(e.key.as_path()),
        })
        .max_by_key(|e| e.key.as_os_str().len())
        .map(|e| e.action)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &Path, action: Action, mode: Mode) -> PathEntry {
        PathEntry {
            path: path.to_string_lossy().into_owned(),
            action,
            mode,
        }
    }

    #[test]
    fn statuses_and_effective_action() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let a = root.join("a");
        let b = a.join("b");
        fs::create_dir_all(&b).unwrap();
        fs::write(root.join("file.txt"), b"").unwrap();

        let entries = vec![
            entry(&a, Action::Watch, Mode::Recursive),  // 0 active
            entry(&b, Action::Ignore, Mode::Recursive), // 1 active, overrides 0
            entry(&b, Action::Watch, Mode::Recursive),  // 2 overridden by 1
            entry(&a, Action::Watch, Mode::Children),   // 3 redundant (0)
            entry(&root.join("nope"), Action::Watch, Mode::Recursive), // 4 not found
            entry(&root.join("file.txt"), Action::Watch, Mode::Recursive), // 5 not dir
        ];
        let (st, active) = evaluate(&entries);
        assert_eq!(st[0], EntryStatus::new(StatusKind::Active, None));
        assert_eq!(st[1], EntryStatus::new(StatusKind::Active, Some(0)));
        assert_eq!(st[2], EntryStatus::new(StatusKind::Overridden, Some(1)));
        assert_eq!(st[3], EntryStatus::new(StatusKind::Redundant, Some(0)));
        assert_eq!(st[4].kind, StatusKind::NotFound);
        assert_eq!(st[5].kind, StatusKind::NotADirectory);
        assert_eq!(active.len(), 2);

        let key = nfc_path(&a.join("x.txt"));
        assert_eq!(effective_action(&key, &active), Some(Action::Watch));
        let key = nfc_path(&b.join("deep/x.txt"));
        assert_eq!(effective_action(&key, &active), Some(Action::Ignore));
        assert_eq!(effective_action(&nfc_path(&root.join("y")), &active), None);
    }

    #[test]
    fn config_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sub/config.toml");
        assert_eq!(Config::load(&path).unwrap(), Config::default());
        let cfg = Config {
            watcher_enabled: false,
            paths: vec![PathEntry {
                path: "~/Downloads".into(),
                action: Action::Watch,
                mode: Mode::Children,
            }],
        };
        cfg.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), cfg);

        // 원본 nfd2nfc 형식(최상위 키 없음)도 읽힌다.
        fs::write(
            &path,
            "[[paths]]\npath = \"~/Documents\"\naction = \"ignore\"\n",
        )
        .unwrap();
        let legacy = Config::load(&path).unwrap();
        assert!(legacy.watcher_enabled);
        assert_eq!(legacy.paths[0].action, Action::Ignore);
        assert_eq!(legacy.paths[0].mode, Mode::Recursive);
    }
}
