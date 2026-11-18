// Rust 커맨드(src-tauri/src/commands.rs)와 1:1 대응하는 타입·호출 래퍼
import { invoke } from "@tauri-apps/api/core";

export type UnicodeForm = "ascii" | "nfc" | "nfd" | "mixed";
export type Action = "watch" | "ignore";
export type Mode = "recursive" | "children";
export type Scope = "name" | "children" | "recursive";
export type Target = "nfc" | "nfd";

export interface WatcherStatus {
  running: boolean;
  watched: number;
}

export interface AppStatus {
  watcher: WatcherStatus;
  fullDiskAccess: boolean | null;
  sessionConverted: number;
  version: string;
  configPath: string;
  logPath: string;
}

export interface PathEntry {
  path: string;
  action: Action;
  mode: Mode;
}

export type StatusKind =
  | "active"
  | "notFound"
  | "notADirectory"
  | "permissionDenied"
  | "redundant"
  | "overridden";

export interface EntryView extends PathEntry {
  display: string;
  status: { kind: StatusKind; reference: number | null };
}

export interface DirItem {
  /** 표시용 (NFC) */
  name: string;
  /** 디스크에 저장된 그대로의 이름 */
  rawName: string;
  path: string;
  isDir: boolean;
  isSymlink: boolean;
  form: UnicodeForm;
  size: number | null;
  /** Unix 초 */
  modified: number | null;
}

export interface DirListing {
  path: string;
  display: string;
  parent: string | null;
  crumbs: { name: string; path: string }[];
  form: UnicodeForm;
  rawName: string;
  items: DirItem[];
}

export interface FormSample {
  path: string;
  relative: string;
  isDir: boolean;
  form: UnicodeForm;
}

export interface FormScan {
  scanned: number;
  ascii: number;
  nfc: number;
  nfd: number;
  mixed: number;
  truncated: boolean;
  samples: FormSample[];
}

export interface AnalyzedItem {
  path: string;
  name: string;
  display: string;
  isDir: boolean;
  form: UnicodeForm;
  nfdCount: number;
  scanned: number;
  error: string | null;
}

export interface Rename {
  from: string;
  to: string;
  isDir: boolean;
}

export interface Report {
  renamed: Rename[];
  errors: { path: string; message: string }[];
  scanned: number;
}

export interface ZipReport {
  output: string;
  files: number;
  dirs: number;
  normalizedNames: number;
  skipped: number;
  bytes: number;
}

export type LogLevel = "convert" | "info" | "warn" | "error";

export interface LogEntry {
  ts: string;
  level: LogLevel;
  message: string;
}

// Tauri는 JS의 camelCase 인자를 Rust의 snake_case 매개변수로 자동 변환한다.
export const api = {
  getStatus: () => invoke<AppStatus>("get_status"),
  setWatching: (enabled: boolean) => invoke<WatcherStatus>("set_watching", { enabled }),
  restartWatcher: () => invoke<WatcherStatus>("restart_watcher"),

  getConfig: () => invoke<EntryView[]>("get_config"),
  evaluateEntries: (entries: PathEntry[]) => invoke<EntryView[]>("evaluate_entries", { entries }),
  saveConfig: (entries: PathEntry[]) => invoke<EntryView[]>("save_config", { entries }),
  suggestedFolders: () => invoke<string[]>("suggested_folders"),

  listDir: (path: string | null, showHidden: boolean) =>
    invoke<DirListing>("list_dir", { path, showHidden }),
  scanTree: (path: string) => invoke<FormScan>("scan_tree", { path }),

  analyzePaths: (paths: string[]) => invoke<AnalyzedItem[]>("analyze_paths", { paths }),
  convertPaths: (paths: string[], scope: Scope, target: Target, dryRun: boolean) =>
    invoke<Report>("convert_paths", { paths, scope, target, dryRun }),
  suggestZipPath: (paths: string[]) => invoke<string | null>("suggest_zip_path", { paths }),
  createZip: (paths: string[], output: string, includeHidden: boolean) =>
    invoke<ZipReport>("create_zip", { paths, output, includeHidden }),

  getLogs: () => invoke<LogEntry[]>("get_logs"),
  clearLogs: () => invoke<void>("clear_logs"),
  revealInFinder: (path: string) => invoke<void>("reveal_in_finder", { path }),
  openLogFolder: () => invoke<void>("open_log_folder"),
  openFdaSettings: () => invoke<void>("open_fda_settings"),
};
