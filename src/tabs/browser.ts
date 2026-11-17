// 탐색기 탭 — 원본 TUI Browser 탭: 폴더를 돌아다니며 각 이름의 NFC/NFD 상태를 보고 바로 변환
import { open } from "@tauri-apps/plugin-dialog";

import { api, type DirListing, type Scope } from "../api";
import type { App, Tab } from "../main";
import { $, busy, errMsg, esc, formBadge, toast } from "../ui";

const REFRESH_MS = 3000;

export class BrowserTab implements Tab {
  readonly title = "탐색기";
  private root!: HTMLElement;
  private listing: DirListing | null = null;
  private selected = -1;
  private showHidden = false;
  private scope: Scope = "recursive";
  private timer: number | undefined;
  private actions: HTMLElement | null = null;

  constructor(private app: App) {}

  mount(root: HTMLElement) {
    this.root = root;
    root.innerHTML = `
      <div class="browser">
        <div class="pathbar">
          <button class="icon-btn" id="br-up" title="상위 폴더 (⌫)">↑</button>
          <button class="icon-btn" id="br-home" title="홈">⌂</button>
          <div class="path-display" id="br-path"></div>
          <button class="btn" id="br-pick">폴더 열기…</button>
        </div>
        <ul class="file-list" id="br-list" tabindex="0"></ul>
        <div class="actionbar" id="br-actionbar">
          <div class="sel-info" id="br-sel">항목을 선택하세요</div>
          <select class="sel" id="br-scope" title="변환 범위">
            <option value="recursive">전체 (하위 포함)</option>
            <option value="children">바로 아래만</option>
            <option value="name">이름만</option>
          </select>
          <button class="btn" id="br-send">변환 목록으로</button>
          <button class="btn primary" id="br-convert">NFC로 변환</button>
        </div>
      </div>`;

    $("#br-up", root).addEventListener("click", () => this.goUp());
    $("#br-home", root).addEventListener("click", () => void this.navigate(null));
    $("#br-pick", root).addEventListener("click", async () => {
      const p = await open({ directory: true, title: "열 폴더 선택" });
      if (typeof p === "string") void this.navigate(p);
    });

    const list = $("#br-list", root);
    list.addEventListener("click", (e) => {
      const li = (e.target as HTMLElement).closest<HTMLElement>("li[data-i]");
      if (!li) return;
      this.select(Number(li.dataset.i));
    });
    list.addEventListener("dblclick", (e) => {
      const li = (e.target as HTMLElement).closest<HTMLElement>("li[data-i]");
      if (li) this.openItem(Number(li.dataset.i));
    });
    list.addEventListener("keydown", (e) => {
      const n = this.listing?.items.length ?? 0;
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        if (!n) return;
        const next = this.selected < 0 ? 0 : this.selected + (e.key === "ArrowDown" ? 1 : -1);
        this.select(Math.max(0, Math.min(n - 1, next)));
      } else if (e.key === "Enter" || (e.key === "ArrowRight" && !e.metaKey)) {
        e.preventDefault();
        if (this.selected >= 0) this.openItem(this.selected);
      } else if (e.key === "Backspace" || e.key === "ArrowLeft") {
        e.preventDefault();
        this.goUp();
      } else if (e.key === ".") {
        this.toggleHidden();
      }
    });

    const scope = $("#br-scope", root) as HTMLSelectElement;
    scope.addEventListener("change", () => (this.scope = scope.value as Scope));

    $("#br-send", root).addEventListener("click", () => {
      const it = this.current();
      if (it) this.app.sendToConvert([it.path]);
    });
    const convert = $("#br-convert", root) as HTMLButtonElement;
    convert.addEventListener("click", () =>
      busy(convert, async () => {
        const it = this.current();
        if (!it) return;
        const scope: Scope = it.isDir ? this.scope : "name";
        const rep = await api.convertPaths([it.path], scope, "nfc", false);
        if (rep.errors.length) toast(`변환 실패: ${rep.errors[0].message}`, "error");
        else toast(rep.renamed.length ? `${rep.renamed.length}개 이름을 변환했습니다` : "바꿀 이름이 없습니다", "success");
        await this.refresh();
        await this.app.refreshStatus();
      }),
    );
  }

  show(actions: HTMLElement) {
    this.actions = actions;
    this.renderActions();
    void this.refresh();
    this.timer = window.setInterval(() => void this.refresh(true), REFRESH_MS);
    $("#br-list", this.root).focus();
  }

  hide() {
    window.clearInterval(this.timer);
    this.actions = null;
  }

  private renderActions() {
    if (!this.actions) return;
    this.actions.innerHTML = `
      <label class="check"><input type="checkbox" id="br-hidden" ${this.showHidden ? "checked" : ""}/> 숨김 파일 표시 <kbd>.</kbd></label>`;
    $("#br-hidden", this.actions).addEventListener("change", () => this.toggleHidden());
  }

  private toggleHidden() {
    this.showHidden = !this.showHidden;
    this.renderActions();
    void this.refresh();
  }

  private current() {
    return this.listing?.items[this.selected];
  }

  private goUp() {
    if (this.listing?.parent) void this.navigate(this.listing.parent, this.listing.path);
  }

  private openItem(i: number) {
    const it = this.listing?.items[i];
    if (it?.isDir) void this.navigate(it.path);
  }

  private async navigate(path: string | null, selectPath?: string) {
    try {
      this.listing = await api.listDir(path, this.showHidden);
      this.selected = selectPath
        ? this.listing.items.findIndex((i) => i.path.normalize("NFC") === selectPath.normalize("NFC"))
        : -1;
      this.render();
      $("#br-list", this.root).scrollTop = 0;
      if (this.selected >= 0) this.scrollToSelected();
    } catch (e) {
      toast(errMsg(e), "error");
    }
  }

  private async refresh(silent = false) {
    const prev = this.current()?.name;
    try {
      const next = await api.listDir(this.listing?.path ?? null, this.showHidden);
      // 변환 후에도 선택 유지: NFC 이름 기준으로 다시 찾는다
      this.listing = next;
      this.selected = prev ? next.items.findIndex((i) => i.name === prev) : -1;
      this.render();
    } catch (e) {
      if (!silent) toast(errMsg(e), "error");
    }
  }

  private select(i: number) {
    this.selected = i;
    this.render();
    this.scrollToSelected();
  }

  private scrollToSelected() {
    $("#br-list", this.root)
      .querySelector(`li[data-i="${this.selected}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }

  private render() {
    const r = this.root;
    const l = this.listing;
    if (!l) return;
    $("#br-path", r).textContent = l.display;
    ($("#br-up", r) as HTMLButtonElement).disabled = !l.parent;

    const nfdCount = l.items.filter((i) => i.form === "nfd" || i.form === "mixed").length;
    $("#br-list", r).innerHTML =
      (l.items.length === 0 ? `<li class="muted empty-row">빈 폴더</li>` : "") +
      l.items
        .map(
          (it, i) => `
        <li data-i="${i}" class="${i === this.selected ? "selected" : ""} ${it.isDir ? "dir" : ""}">
          <span class="fi">${it.isDir ? "📁" : "📄"}</span>
          <span class="name">${esc(it.name)}</span>
          ${formBadge(it.form)}
        </li>`,
        )
        .join("");

    const it = this.current();
    $("#br-sel", r).innerHTML = it
      ? `<b>${esc(it.name)}</b> ${formBadge(it.form)}`
      : `항목 ${l.items.length}개${nfdCount ? ` · <span class="status warn">NFD ${nfdCount}개</span>` : ""}`;
    const scope = $("#br-scope", r) as HTMLSelectElement;
    scope.disabled = !it?.isDir;
    if (!it?.isDir) scope.value = "name";
    else scope.value = this.scope;
    ($("#br-convert", r) as HTMLButtonElement).disabled = !it;
    ($("#br-send", r) as HTMLButtonElement).disabled = !it;
  }
}
