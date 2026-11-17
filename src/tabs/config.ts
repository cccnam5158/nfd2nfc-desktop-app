// 감시 폴더 탭 — 원본 TUI Config 탭(규칙 테이블 + 상태 + 설명 박스)을 GUI로 옮긴 것
import { open } from "@tauri-apps/plugin-dialog";

import { api, type Action, type EntryView, type Mode, type PathEntry, type StatusKind } from "../api";
import type { App, Tab } from "../main";
import { $, busy, errMsg, esc, toast } from "../ui";

const STATUS_META: Record<StatusKind, { sym: string; label: string; cls: string }> = {
  active: { sym: "✓", label: "적용 중", cls: "ok" },
  redundant: { sym: "~", label: "중복", cls: "muted" },
  overridden: { sym: "!", label: "무시됨", cls: "warn" },
  notFound: { sym: "✗", label: "경로 없음", cls: "err" },
  notADirectory: { sym: "✗", label: "폴더 아님", cls: "err" },
  permissionDenied: { sym: "✗", label: "권한 없음", cls: "err" },
};

function describe(v: EntryView): string {
  const ref = v.status.reference != null ? `#${v.status.reference + 1}` : "";
  switch (v.status.kind) {
    case "active": {
      const base =
        v.action === "ignore"
          ? "이 폴더와 모든 하위 폴더를 감시 대상에서 제외합니다."
          : v.mode === "recursive"
            ? "이 폴더와 모든 하위 폴더를 감시하다가 NFD 이름이 생기면 NFC로 자동 변환합니다."
            : "이 폴더 바로 아래 항목만 감시합니다. 하위 폴더 안쪽은 감시하지 않습니다.";
      return ref ? `${ref} 규칙을 덮어씁니다. ${base}` : base;
    }
    case "redundant":
      return `${ref} 규칙이 이미 같은 동작으로 이 경로를 포함하고 있어 적용되지 않습니다.`;
    case "overridden":
      return `같은 경로의 ${ref} 규칙이 우선하므로 이 규칙은 적용되지 않습니다.`;
    case "notFound":
      return "경로를 찾을 수 없습니다. 폴더가 이동되었거나 외장 디스크가 연결되지 않았을 수 있습니다.";
    case "notADirectory":
      return "폴더가 아닙니다. 감시 규칙에는 폴더만 넣을 수 있습니다.";
    case "permissionDenied":
      return "접근 권한이 없습니다. 홈 탭에서 ‘전체 디스크 접근 권한’을 허용하세요.";
  }
}

export class ConfigTab implements Tab {
  readonly title = "감시 폴더";
  readonly dropHint = "놓으면 감시 폴더로 추가합니다";
  private root!: HTMLElement;
  private entries: PathEntry[] = [];
  private views: EntryView[] = [];
  private saved = "[]";
  private selected = -1;
  private loaded = false;
  private actions: HTMLElement | null = null;

  constructor(private app: App) {}

  private get dirty() {
    return JSON.stringify(this.entries) !== this.saved;
  }

  mount(root: HTMLElement) {
    this.root = root;
    root.innerHTML = `
      <div class="config">
        <div class="table-wrap">
          <table class="rules">
            <thead><tr>
              <th class="c-num">#</th><th>경로</th><th class="c-action">동작</th>
              <th class="c-mode">범위</th><th class="c-status">상태</th><th class="c-ops"></th>
            </tr></thead>
            <tbody id="cfg-body"></tbody>
          </table>
          <div class="empty" id="cfg-empty" hidden>
            <div class="empty-ico">◉</div>
            <p>아직 감시 중인 폴더가 없습니다.<br/>파일을 주고받는 폴더를 추가하세요. 폴더를 이 창에 끌어다 놓아도 됩니다.</p>
            <div class="row gap center">
              <button class="btn primary" id="cfg-empty-add">폴더 추가…</button>
              <button class="btn" id="cfg-suggest">데스크탑·다운로드·문서 추가</button>
            </div>
          </div>
        </div>
        <div class="desc-box" id="cfg-desc">규칙을 선택하면 설명이 표시됩니다.</div>
        <p class="hint">감시는 <b>새로 생기거나 이름이 바뀐</b> 항목에만 적용됩니다. 이미 있는 파일은 각 행의
          <b>기존 파일 정리</b> 버튼을 누르거나 ‘변환 · 압축’ 탭에서 한 번에 바꾸세요.</p>
      </div>`;

    $("#cfg-empty-add", root).addEventListener("click", () => void this.addViaDialog());
    $("#cfg-suggest", root).addEventListener("click", async () => {
      const folders = await api.suggestedFolders();
      this.addPaths(folders);
    });

    const body = $("#cfg-body", root);
    body.addEventListener("click", (e) => this.onTableClick(e));
    body.addEventListener("change", (e) => this.onTableChange(e));

    window.addEventListener("keydown", (e) => {
      if (this.app.current !== "config") return;
      const tag = (e.target as HTMLElement).tagName;
      if (tag === "INPUT" || tag === "SELECT") return;
      if (e.metaKey && e.key === "s") {
        e.preventDefault();
        void this.save();
      } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const n = this.entries.length;
        if (!n) return;
        this.selected = Math.max(0, Math.min(n - 1, this.selected + (e.key === "ArrowDown" ? 1 : -1)));
        this.render();
      } else if ((e.key === "Backspace" || e.key === "Delete") && this.selected >= 0) {
        e.preventDefault();
        this.remove(this.selected);
      }
    });
  }

  show(actions: HTMLElement) {
    this.actions = actions;
    this.renderActions();
    if (!this.loaded) void this.load();
    else void this.reevaluate(); // 폴더가 생기거나 사라졌을 수 있다
  }

  hide() {
    this.actions = null;
  }

  onDrop(paths: string[]) {
    this.addPaths(paths);
  }

  // ───────── 데이터 ─────────

  private async load() {
    try {
      const views = await api.getConfig();
      this.applyViews(views, true);
      this.loaded = true;
    } catch (e) {
      toast(errMsg(e), "error");
    }
  }

  private applyViews(views: EntryView[], asSaved = false) {
    this.views = views;
    this.entries = views.map(({ path, action, mode }) => ({ path, action, mode }));
    if (asSaved) this.saved = JSON.stringify(this.entries);
    if (this.selected >= this.entries.length) this.selected = this.entries.length - 1;
    this.render();
  }

  private async reevaluate() {
    try {
      const views = await api.evaluateEntries(this.entries);
      this.views = views;
      this.render();
    } catch (e) {
      toast(errMsg(e), "error");
    }
  }

  private edit(mutator: () => void) {
    mutator();
    this.render();
    void this.reevaluate();
  }

  private addPaths(paths: string[]) {
    const existing = new Set(this.entries.map((e) => e.path.normalize("NFC")));
    const fresh = paths.filter((p) => !existing.has(p.normalize("NFC")));
    if (!fresh.length) {
      toast("이미 목록에 있는 폴더입니다", "info");
      return;
    }
    this.edit(() => {
      for (const p of fresh) this.entries.push({ path: p, action: "watch", mode: "recursive" });
      this.selected = this.entries.length - 1;
    });
    toast(`${fresh.length}개 폴더를 추가했습니다 — 저장하면 적용됩니다`, "info");
  }

  private async addViaDialog() {
    const picked = await open({ directory: true, multiple: true, title: "감시할 폴더 선택" });
    if (!picked) return;
    this.addPaths(Array.isArray(picked) ? picked : [picked]);
  }

  private remove(i: number) {
    this.edit(() => {
      this.entries.splice(i, 1);
      this.views.splice(i, 1);
      this.selected = Math.min(i, this.entries.length - 1);
    });
  }

  private move(i: number, delta: number) {
    const j = i + delta;
    if (j < 0 || j >= this.entries.length) return;
    this.edit(() => {
      [this.entries[i], this.entries[j]] = [this.entries[j], this.entries[i]];
      [this.views[i], this.views[j]] = [this.views[j], this.views[i]];
      this.selected = j;
    });
  }

  private sort() {
    this.edit(() => {
      const pairs = this.entries.map((e, i) => ({ e, v: this.views[i] }));
      pairs.sort((a, b) => a.e.path.localeCompare(b.e.path, "ko"));
      this.entries = pairs.map((p) => p.e);
      this.views = pairs.map((p) => p.v);
      this.selected = -1;
    });
  }

  private async revert() {
    this.entries = JSON.parse(this.saved);
    this.selected = -1;
    await this.reevaluate();
    toast("저장된 설정으로 되돌렸습니다", "info");
  }

  private async save() {
    if (!this.dirty) return;
    const btn = this.actions?.querySelector<HTMLButtonElement>("#cfg-save");
    const run = async () => {
      const views = await api.saveConfig(this.entries);
      this.applyViews(views, true);
      await this.app.refreshStatus();
      toast("저장했습니다. 감시 중이면 새 설정으로 다시 시작합니다", "success");
    };
    if (btn) await busy(btn, run);
    else await run().catch((e) => toast(errMsg(e), "error"));
  }

  // ───────── 이벤트 ─────────

  private onTableClick(e: Event) {
    const t = e.target as HTMLElement;
    const row = t.closest<HTMLElement>("tr[data-i]");
    if (!row) return;
    const i = Number(row.dataset.i);
    const op = t.closest<HTMLElement>("[data-op]")?.dataset.op;
    switch (op) {
      case "up":
        return this.move(i, -1);
      case "down":
        return this.move(i, 1);
      case "del":
        return this.remove(i);
      case "scan":
        return this.app.sendToConvert([this.views[i]?.path ?? this.entries[i].path]);
    }
    if (t.tagName !== "SELECT" && this.selected !== i) {
      this.selected = i;
      this.render();
    }
  }

  private onTableChange(e: Event) {
    const t = e.target as HTMLSelectElement;
    const row = t.closest<HTMLElement>("tr[data-i]");
    if (!row) return;
    const i = Number(row.dataset.i);
    this.edit(() => {
      this.selected = i;
      if (t.dataset.field === "action") {
        this.entries[i].action = t.value as Action;
        // ignore는 항상 recursive
        if (t.value === "ignore") this.entries[i].mode = "recursive";
      } else if (t.dataset.field === "mode") {
        this.entries[i].mode = t.value as Mode;
      }
    });
  }

  // ───────── 렌더링 ─────────

  private renderActions() {
    const a = this.actions;
    if (!a) return;
    a.innerHTML = `
      ${this.dirty ? `<span class="pill warn">저장 안 됨</span>` : ""}
      <button class="btn" id="cfg-add">＋ 폴더 추가</button>
      <button class="btn" id="cfg-sort" ${this.entries.length < 2 ? "disabled" : ""}>이름순 정렬</button>
      <button class="btn" id="cfg-revert" ${this.dirty ? "" : "disabled"}>되돌리기</button>
      <button class="btn primary" id="cfg-save" ${this.dirty ? "" : "disabled"} title="⌘S">저장</button>`;
    $("#cfg-add", a).addEventListener("click", () => void this.addViaDialog());
    $("#cfg-sort", a).addEventListener("click", () => this.sort());
    $("#cfg-revert", a).addEventListener("click", () => void this.revert());
    $("#cfg-save", a).addEventListener("click", () => void this.save());
  }

  private render() {
    const r = this.root;
    const n = this.entries.length;
    $("#cfg-empty", r).hidden = n > 0;
    ($("table.rules", r) as HTMLElement).hidden = n === 0;

    $("#cfg-body", r).innerHTML = this.entries
      .map((e, i) => {
        const v = this.views[i];
        const kind = v?.status.kind;
        const meta = kind ? STATUS_META[kind] : null;
        const inactive = kind && kind !== "active";
        const display = v?.path === e.path ? v.display : e.path;
        const ignore = e.action === "ignore";
        return `
          <tr data-i="${i}" class="${i === this.selected ? "selected" : ""} ${inactive ? "inactive" : ""}">
            <td class="c-num">${i + 1}</td>
            <td class="c-path" title="${esc(e.path)}">${esc(display.normalize("NFC"))}</td>
            <td class="c-action">
              <select data-field="action" class="sel ${ignore ? "sel-ignore" : "sel-watch"}">
                <option value="watch" ${!ignore ? "selected" : ""}>감시</option>
                <option value="ignore" ${ignore ? "selected" : ""}>제외</option>
              </select>
            </td>
            <td class="c-mode">
              <select data-field="mode" class="sel" ${ignore ? "disabled" : ""}>
                <option value="recursive" ${e.mode === "recursive" ? "selected" : ""}>하위 전체</option>
                <option value="children" ${e.mode === "children" && !ignore ? "selected" : ""}>바로 아래만</option>
              </select>
            </td>
            <td class="c-status">${meta ? `<span class="status ${meta.cls}">${meta.sym} ${meta.label}</span>` : `<span class="status muted">…</span>`}</td>
            <td class="c-ops">
              ${!ignore && kind === "active" ? `<button class="icon-btn text" data-op="scan" title="이 폴더에 이미 있는 NFD 파일을 변환 목록으로 보냅니다">기존 파일 정리</button>` : ""}
              <button class="icon-btn" data-op="up" title="위로" ${i === 0 ? "disabled" : ""}>↑</button>
              <button class="icon-btn" data-op="down" title="아래로" ${i === n - 1 ? "disabled" : ""}>↓</button>
              <button class="icon-btn danger" data-op="del" title="삭제">✕</button>
            </td>
          </tr>`;
      })
      .join("");

    const sel = this.views[this.selected];
    $("#cfg-desc", r).innerHTML =
      sel && this.selected >= 0
        ? `<code>${esc(this.entries[this.selected].path.normalize("NFC"))}</code><p>${esc(describe(sel))}</p>`
        : `<p class="muted">규칙을 선택하면 설명이 표시됩니다. 위에 있는 규칙일수록 우선합니다(같은 경로일 때).
             감시 폴더 안의 특정 하위 폴더만 빼려면 그 폴더를 <b>제외</b>로 추가하세요.</p>`;

    this.renderActions();
  }
}
