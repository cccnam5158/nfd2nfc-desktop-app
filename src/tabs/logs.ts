// 로그 탭 — 감시자·수동 변환·오류 기록. 맨 아래에 있으면 자동 스크롤
import { api, type LogEntry, type LogLevel } from "../api";
import type { App, Tab } from "../main";
import { $, esc, toast } from "../ui";

const LEVEL_LABEL: Record<LogLevel, string> = {
  convert: "변환",
  info: "정보",
  warn: "경고",
  error: "오류",
};

type Filter = "all" | "convert" | "problem";

export class LogsTab implements Tab {
  readonly title = "로그";
  private root!: HTMLElement;
  private entries: LogEntry[] = [];
  private filter: Filter = "all";
  private loaded = false;
  private visible = false;
  private actions: HTMLElement | null = null;

  constructor(private app: App) {}

  mount(root: HTMLElement) {
    this.root = root;
    root.innerHTML = `
      <div class="logs">
        <div class="log-list" id="lg-list"></div>
        <button class="jump" id="lg-jump" hidden>↓ 최신 로그로</button>
      </div>`;
    const list = $("#lg-list", root);
    list.addEventListener("scroll", () => ($("#lg-jump", root).hidden = this.atBottom()));
    $("#lg-jump", root).addEventListener("click", () => this.scrollToBottom());
    list.addEventListener("dblclick", (e) => {
      const row = (e.target as HTMLElement).closest<HTMLElement>(".log-row[data-path]");
      if (row) void api.revealInFinder(row.dataset.path!).catch((err) => toast(String(err), "error"));
    });

    this.app.onLog((e) => {
      this.entries.push(e);
      if (this.entries.length > 5000) this.entries.shift();
      if (this.visible) this.append(e);
    });
  }

  async show(actions: HTMLElement) {
    this.visible = true;
    this.actions = actions;
    this.renderActions();
    if (!this.loaded) {
      this.entries = await api.getLogs();
      this.loaded = true;
    }
    this.render();
    this.scrollToBottom();
  }

  hide() {
    this.visible = false;
    this.actions = null;
  }

  private renderActions() {
    const a = this.actions;
    if (!a) return;
    a.innerHTML = `
      <div class="segmented small" id="lg-filter">
        <button data-v="all">전체</button>
        <button data-v="convert">변환</button>
        <button data-v="problem">경고·오류</button>
      </div>
      <span class="muted small" id="lg-count"></span>
      <button class="btn" id="lg-folder">로그 파일 보기</button>
      <button class="btn" id="lg-clear">지우기</button>`;
    a.querySelectorAll<HTMLButtonElement>("#lg-filter button").forEach((b) => {
      b.classList.toggle("active", b.dataset.v === this.filter);
      b.addEventListener("click", () => {
        this.filter = b.dataset.v as Filter;
        this.renderActions();
        this.render();
        this.scrollToBottom();
      });
    });
    $("#lg-folder", a).addEventListener("click", () => void api.openLogFolder());
    $("#lg-clear", a).addEventListener("click", async () => {
      await api.clearLogs();
      this.entries = [];
      this.render();
      toast("로그를 지웠습니다", "success");
    });
    this.updateCount();
  }

  private matches(e: LogEntry) {
    if (this.filter === "convert") return e.level === "convert";
    if (this.filter === "problem") return e.level === "warn" || e.level === "error";
    return true;
  }

  private row(e: LogEntry) {
    // 메시지 안의 `~/…` 경로 (뒤에 " — 오류 설명"이 붙을 수 있음)
    const path = /(~\/.+?)(?:\s—\s|$)/.exec(e.message)?.[1];
    // 날짜는 MM-DD만 (원본 TUI와 동일한 표시)
    return `<div class="log-row lv-${e.level} ${path ? "has-path" : ""}" ${path ? `data-path="${esc(path)}" title="더블클릭: Finder에서 보기"` : ""}>
      <span class="ts">${esc(e.ts.slice(5))}</span>
      <span class="lv">${LEVEL_LABEL[e.level]}</span>
      <span class="msg">${esc(e.message.normalize("NFC"))}</span>
    </div>`;
  }

  private render() {
    const shown = this.entries.filter((e) => this.matches(e));
    $("#lg-list", this.root).innerHTML = shown.length
      ? shown.map((e) => this.row(e)).join("")
      : `<div class="empty"><div class="empty-ico">≡</div><p>표시할 로그가 없습니다.</p></div>`;
    this.updateCount();
  }

  private append(e: LogEntry) {
    if (!this.matches(e)) return this.updateCount();
    const list = $("#lg-list", this.root);
    const stick = this.atBottom();
    list.querySelector(".empty")?.remove();
    list.insertAdjacentHTML("beforeend", this.row(e));
    while (list.children.length > 5000) list.firstElementChild?.remove();
    if (stick) this.scrollToBottom();
    this.updateCount();
  }

  private updateCount() {
    const el = this.actions?.querySelector("#lg-count");
    if (el) el.textContent = `${this.entries.filter((e) => this.matches(e)).length.toLocaleString()}개`;
  }

  private atBottom() {
    const l = $("#lg-list", this.root);
    return l.scrollHeight - l.scrollTop - l.clientHeight < 24;
  }

  private scrollToBottom() {
    const l = $("#lg-list", this.root);
    l.scrollTop = l.scrollHeight;
    $("#lg-jump", this.root).hidden = true;
  }
}
