import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";

import { api, type AppStatus, type LogEntry, type WatcherStatus } from "./api";
import { $, $$, errMsg, toast } from "./ui";
import { HomeTab } from "./tabs/home";
import { ConfigTab } from "./tabs/config";
import { ConvertTab } from "./tabs/convert";
import { BrowserTab } from "./tabs/browser";
import { LogsTab } from "./tabs/logs";

export type TabId = "home" | "config" | "convert" | "browser" | "logs";

export interface Tab {
  readonly title: string;
  /** 처음 한 번 DOM을 만든다 */
  mount(root: HTMLElement): void;
  /** 탭이 보일 때마다 호출 (상단 바 액션 렌더링 포함) */
  show?(actions: HTMLElement): void;
  hide?(): void;
  /** 드래그앤드롭으로 들어온 경로 */
  onDrop?(paths: string[]): void;
  /** 드롭 오버레이에 표시할 안내 문구 */
  dropHint?: string;
}

/** 탭 간에 공유하는 앱 전역 상태와 이벤트 허브 */
export class App {
  status: AppStatus | null = null;
  readonly tabs: Record<TabId, Tab>;
  current: TabId = "home";
  private statusListeners: ((s: AppStatus) => void)[] = [];
  private logListeners: ((e: LogEntry) => void)[] = [];

  constructor() {
    this.tabs = {
      home: new HomeTab(this),
      config: new ConfigTab(this),
      convert: new ConvertTab(this),
      browser: new BrowserTab(this),
      logs: new LogsTab(this),
    };
  }

  onStatus(fn: (s: AppStatus) => void) {
    this.statusListeners.push(fn);
    if (this.status) fn(this.status);
  }

  onLog(fn: (e: LogEntry) => void) {
    this.logListeners.push(fn);
  }

  async refreshStatus() {
    try {
      this.setStatus(await api.getStatus());
    } catch (e) {
      toast(errMsg(e), "error");
    }
  }

  setStatus(s: AppStatus) {
    this.status = s;
    renderSidebarStatus(s.watcher);
    renderPermBanner(s);
    this.statusListeners.forEach((fn) => fn(s));
  }

  patchWatcher(w: WatcherStatus) {
    if (this.status) this.setStatus({ ...this.status, watcher: w });
    else void this.refreshStatus();
  }

  switchTo(id: TabId) {
    if (this.current !== id) this.tabs[this.current].hide?.();
    this.current = id;
    $$("#nav button").forEach((b) => b.classList.toggle("active", b.dataset.tab === id));
    $$(".page").forEach((p) => (p.hidden = p.id !== `page-${id}`));
    $("#page-title").textContent = this.tabs[id].title;
    const actions = $("#page-actions");
    actions.innerHTML = "";
    this.tabs[id].show?.(actions);
  }

  /** 다른 탭에서 "변환 목록으로 보내기" */
  sendToConvert(paths: string[]) {
    this.switchTo("convert");
    this.tabs.convert.onDrop?.(paths);
  }

  emitLog(e: LogEntry) {
    this.logListeners.forEach((fn) => fn(e));
  }
}

function renderSidebarStatus(w: WatcherStatus) {
  const el = $("#sidebar-status");
  el.classList.toggle("running", w.running);
  $(".label", el).textContent = w.running ? `감시 중 · 폴더 ${w.watched}개` : "감시 중지됨";
}

let permDismissed = false;

/** 감시 중인데 전체 디스크 접근 권한이 없으면 모든 화면 상단에 경고 */
function renderPermBanner(s: AppStatus) {
  const show = !permDismissed && s.watcher.running && s.watcher.watched > 0 && s.fullDiskAccess === false;
  $("#perm-banner").hidden = !show;
  $("#sidebar-status").classList.toggle("warn", s.watcher.running && s.fullDiskAccess === false);
  if (s.watcher.running && s.fullDiskAccess === false)
    $(".label", $("#sidebar-status")).textContent = `감시 중 · 권한 필요`;
}

async function main() {
  const app = new App();

  for (const [id, tab] of Object.entries(app.tabs)) tab.mount($(`#page-${id}`));

  $$("#nav button").forEach((b) =>
    b.addEventListener("click", () => app.switchTo(b.dataset.tab as TabId)),
  );

  // ⌘1 ~ ⌘5 탭 전환
  const order: TabId[] = ["home", "config", "convert", "browser", "logs"];
  window.addEventListener("keydown", (e) => {
    if (e.metaKey && !e.shiftKey && !e.altKey && /^[1-5]$/.test(e.key)) {
      e.preventDefault();
      app.switchTo(order[Number(e.key) - 1]);
    }
  });

  // 백엔드 이벤트
  await listen<WatcherStatus>("watcher-status", (e) => app.patchWatcher(e.payload));
  await listen<number>("watcher-converted", (e) => {
    if (app.status) app.setStatus({ ...app.status, sessionConverted: e.payload });
  });
  await listen<LogEntry>("log-entry", (e) => app.emitLog(e.payload));

  // 드래그앤드롭 — 현재 탭이 처리할 수 있으면 그 탭으로, 아니면 변환 탭으로
  const overlay = $("#drop-overlay");
  await getCurrentWebview().onDragDropEvent((event) => {
    const p = event.payload;
    if (p.type === "enter" || p.type === "over") {
      const target = app.tabs[app.current].onDrop ? app.tabs[app.current] : app.tabs.convert;
      $("#drop-overlay-text").textContent = target.dropHint ?? "놓으면 변환 목록에 추가합니다";
      overlay.hidden = false;
    } else if (p.type === "leave") {
      overlay.hidden = true;
    } else if (p.type === "drop") {
      overlay.hidden = true;
      if (!p.paths.length) return;
      if (app.tabs[app.current].onDrop) app.tabs[app.current].onDrop!(p.paths);
      else app.sendToConvert(p.paths);
    }
  });

  $("#perm-open").addEventListener("click", () => void api.openFdaSettings());
  $("#perm-relaunch").addEventListener("click", () => void api.relaunchApp());
  $("#perm-dismiss").addEventListener("click", () => {
    permDismissed = true;
    $("#perm-banner").hidden = true;
  });

  app.switchTo("home");
  await app.refreshStatus();
  // 권한 상태 등은 앱 밖(시스템 설정)에서 바뀔 수 있으니 창이 다시 활성화될 때 갱신
  window.addEventListener("focus", () => void app.refreshStatus());
}

main().catch((e) => {
  console.error(e);
  toast(errMsg(e), "error");
});
