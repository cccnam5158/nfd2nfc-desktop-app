// 탐색기 탭
// - 폴더 한 번 클릭 → 하위로 이동(drill down), 경로 조각/뒤로/앞으로/상위 이동
// - 각 이름의 NFC/NFD 상태 배지, 필터·검색, 형태별 개수
// - 상세 패널: Windows에서 보이는 이름, 글자별 코드 포인트, 하위 트리 전체 검사
import { open } from "@tauri-apps/plugin-dialog";

import { api, type DirItem, type DirListing, type FormScan, type Scope, type UnicodeForm } from "../api";
import type { App, Tab } from "../main";
import {
  $,
  analyzeName,
  busy,
  errMsg,
  esc,
  formatBytes,
  formatDate,
  formBadge,
  FORM_LABEL,
  toast,
  utf8Len,
  visualizeNfd,
} from "../ui";

const REFRESH_MS = 3000;
const SCAN_TTL_MS = 30_000;
const SAMPLE_PAGE = 100;

type Filter = "all" | "nfd" | "nfc" | "ascii";

/** 상세 패널에 표시할 대상 */
interface Subject {
  path: string;
  name: string;
  rawName: string;
  isDir: boolean;
  form: UnicodeForm;
  size: number | null;
  modified: number | null;
  /** 지금 들어와 있는 폴더 자신인지 */
  isCurrent: boolean;
}

const needsFix = (f: UnicodeForm) => f === "nfd" || f === "mixed";
const same = (a: string, b: string) => a.normalize("NFC") === b.normalize("NFC");
const parentOf = (p: string) => p.slice(0, p.lastIndexOf("/")) || "/";

export class BrowserTab implements Tab {
  readonly title = "탐색기";
  readonly dropHint = "놓으면 탐색기에서 엽니다";
  private root!: HTMLElement;
  private listing: DirListing | null = null;
  /** 선택한 항목 경로 (없으면 현재 폴더가 상세 대상) */
  private selected: string | null = null;
  private showHidden = false;
  private filter: Filter = "all";
  private query = "";
  private scope: Scope = "recursive";
  private back: string[] = [];
  private forward: string[] = [];
  private timer: number | undefined;
  private actions: HTMLElement | null = null;
  private scans = new Map<string, { scan: FormScan; at: number }>();
  private scanning = new Set<string>();
  private samplesShown = SAMPLE_PAGE;
  private detailKey = "";

  constructor(private app: App) {}

  mount(root: HTMLElement) {
    this.root = root;
    root.innerHTML = `
      <div class="browser">
        <div class="br-toolbar">
          <div class="btn-group">
            <button class="icon-btn" id="br-back" title="뒤로 (⌘[)">‹</button>
            <button class="icon-btn" id="br-fwd" title="앞으로 (⌘])">›</button>
            <button class="icon-btn" id="br-up" title="상위 폴더 (⌫)">↑</button>
            <button class="icon-btn" id="br-home" title="홈">⌂</button>
          </div>
          <nav class="crumbs" id="br-crumbs"></nav>
          <button class="btn" id="br-pick">폴더 열기…</button>
        </div>

        <div class="br-filterbar">
          <input type="search" class="search" id="br-search" placeholder="이름 검색  ⌘F" spellcheck="false" />
          <div class="segmented small" id="br-filter">
            <button data-v="all">전체 <span data-count="all"></span></button>
            <button data-v="nfd">NFD·혼합 <span data-count="nfd"></span></button>
            <button data-v="nfc">NFC <span data-count="nfc"></span></button>
            <button data-v="ascii">ASCII <span data-count="ascii"></span></button>
          </div>
        </div>

        <div class="br-body">
          <div class="br-list-wrap">
            <div class="br-head">
              <span></span><span>이름</span><span>상태</span><span class="r">크기</span><span>수정일</span><span></span>
            </div>
            <ul class="file-list" id="br-list" tabindex="0"></ul>
          </div>
          <aside class="br-detail" id="br-detail"></aside>
        </div>
      </div>`;

    $("#br-back", root).addEventListener("click", () => this.goBack());
    $("#br-fwd", root).addEventListener("click", () => this.goForward());
    $("#br-up", root).addEventListener("click", () => this.goUp());
    $("#br-home", root).addEventListener("click", () => void this.navigate(null));
    $("#br-pick", root).addEventListener("click", async () => {
      const p = await open({ directory: true, title: "열 폴더 선택" });
      if (typeof p === "string") void this.navigate(p);
    });
    $("#br-crumbs", root).addEventListener("click", (e) => {
      const c = (e.target as HTMLElement).closest<HTMLElement>("[data-path]");
      if (c) void this.navigate(c.dataset.path!);
    });

    const search = $("#br-search", root) as HTMLInputElement;
    search.addEventListener("input", () => {
      this.query = search.value.trim().normalize("NFC").toLowerCase();
      this.renderList();
    });
    search.addEventListener("keydown", (e) => {
      if (e.key === "Escape") {
        search.value = "";
        this.query = "";
        this.renderList();
        $("#br-list", root).focus();
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        $("#br-list", root).focus();
        this.moveSelection(1);
      }
    });

    $("#br-filter", root).addEventListener("click", (e) => {
      const b = (e.target as HTMLElement).closest<HTMLElement>("button[data-v]");
      if (!b) return;
      this.filter = b.dataset.v as Filter;
      this.renderList();
    });

    const list = $("#br-list", root);
    list.addEventListener("click", (e) => {
      const t = e.target as HTMLElement;
      const li = t.closest<HTMLElement>("li[data-path]");
      if (!li) return;
      const item = this.itemByPath(li.dataset.path!);
      if (!item) return;
      // ⓘ: 폴더에 들어가지 않고 정보만 보기
      if (t.closest("[data-op=info]")) return this.select(item.path);
      if (item.isDir) void this.navigate(item.path);
      else this.select(item.path);
    });
    list.addEventListener("keydown", (e) => this.onListKey(e));

    $("#br-detail", root).addEventListener("click", (e) => this.onDetailClick(e));

    window.addEventListener("keydown", (e) => {
      if (this.app.current !== "browser" || !e.metaKey) return;
      if (e.key === "[") {
        e.preventDefault();
        this.goBack();
      } else if (e.key === "]") {
        e.preventDefault();
        this.goForward();
      } else if (e.key === "f") {
        e.preventDefault();
        search.focus();
        search.select();
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        this.goUp();
      }
    });
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

  /** 폴더는 열고, 파일은 그 폴더로 가서 선택한다. */
  onDrop(paths: string[]) {
    const first = paths[0];
    if (!first) return;
    void (async () => {
      const isDir = await api.listDir(first, this.showHidden).then(
        () => true,
        () => false,
      );
      if (isDir) await this.navigate(first);
      else await this.navigate(parentOf(first), { select: first });
    })();
  }

  // ───────── 탐색 ─────────

  private async navigate(path: string | null, opts: { history?: boolean; select?: string } = {}) {
    const listing = await api.listDir(path, this.showHidden).catch((e) => {
      toast(errMsg(e), "error");
      return null;
    });
    if (!listing) return;
    if (opts.history !== false && this.listing && !same(this.listing.path, listing.path)) {
      this.back.push(this.listing.path);
      this.forward = [];
    }
    this.listing = listing;
    this.selected = opts.select ?? null;
    this.samplesShown = SAMPLE_PAGE;
    this.renderAll();
    $("#br-list", this.root).scrollTop = 0;
    if (this.selected) this.scrollToSelected();
  }

  private goBack() {
    const prev = this.back.pop();
    if (!prev || !this.listing) return;
    this.forward.push(this.listing.path);
    void this.navigate(prev, { history: false, select: this.listing.path });
  }

  private goForward() {
    const next = this.forward.pop();
    if (!next || !this.listing) return;
    this.back.push(this.listing.path);
    void this.navigate(next, { history: false });
  }

  private goUp() {
    if (this.listing?.parent) void this.navigate(this.listing.parent, { select: this.listing.path });
  }

  private async refresh(silent = false) {
    try {
      const next = await api.listDir(this.listing?.path ?? null, this.showHidden);
      const changed = JSON.stringify(next) !== JSON.stringify(this.listing);
      this.listing = next;
      // 변환으로 이름 형태가 바뀌어도 NFC 기준으로 선택을 유지
      if (this.selected && !next.items.some((i) => same(i.path, this.selected!))) this.selected = null;
      if (changed || !silent) this.renderAll();
    } catch (e) {
      if (!silent) toast(errMsg(e), "error");
    }
  }

  private toggleHidden() {
    this.showHidden = !this.showHidden;
    this.renderActions();
    void this.refresh();
  }

  // ───────── 선택 ─────────

  private visibleItems(): DirItem[] {
    const items = this.listing?.items ?? [];
    return items.filter((i) => {
      if (this.query && !i.name.toLowerCase().includes(this.query)) return false;
      switch (this.filter) {
        case "nfd":
          return needsFix(i.form);
        case "nfc":
          return i.form === "nfc";
        case "ascii":
          return i.form === "ascii";
        default:
          return true;
      }
    });
  }

  private itemByPath(path: string) {
    return this.listing?.items.find((i) => same(i.path, path));
  }

  private select(path: string | null) {
    this.selected = path;
    this.samplesShown = SAMPLE_PAGE;
    this.renderList();
    this.renderDetail();
    this.scrollToSelected();
  }

  private moveSelection(delta: number) {
    const items = this.visibleItems();
    if (!items.length) return;
    const idx = this.selected ? items.findIndex((i) => same(i.path, this.selected!)) : -1;
    const next = idx < 0 ? (delta > 0 ? 0 : items.length - 1) : Math.max(0, Math.min(items.length - 1, idx + delta));
    this.select(items[next].path);
  }

  private onListKey(e: KeyboardEvent) {
    const cur = this.selected ? this.itemByPath(this.selected) : undefined;
    switch (e.key) {
      case "ArrowDown":
      case "ArrowUp":
        e.preventDefault();
        this.moveSelection(e.key === "ArrowDown" ? 1 : -1);
        break;
      case "Enter":
      case "ArrowRight":
        if (e.metaKey) return;
        e.preventDefault();
        if (cur?.isDir) void this.navigate(cur.path);
        break;
      case "Backspace":
      case "ArrowLeft":
        if (e.metaKey) return;
        e.preventDefault();
        this.goUp();
        break;
      case "Escape":
        this.select(null);
        break;
      case ".":
        this.toggleHidden();
        break;
    }
  }

  private scrollToSelected() {
    if (!this.selected) return;
    const el = Array.from($("#br-list", this.root).querySelectorAll<HTMLElement>("li[data-path]")).find((li) =>
      same(li.dataset.path!, this.selected!),
    );
    el?.scrollIntoView({ block: "nearest" });
  }

  // ───────── 상세 대상 · 검사 ─────────

  private subject(): Subject | null {
    const l = this.listing;
    if (!l) return null;
    const sel = this.selected ? this.itemByPath(this.selected) : undefined;
    if (sel) return { ...sel, isCurrent: false };
    const last = l.crumbs[l.crumbs.length - 1];
    return {
      path: l.path,
      name: last?.name ?? l.display,
      rawName: l.rawName || last?.name || "/",
      isDir: true,
      form: l.form,
      size: null,
      modified: null,
      isCurrent: true,
    };
  }

  /**
   * 홈·루트·최상위 볼륨은 항목이 수십만 개일 수 있고, 데스크탑·문서 등 보호 폴더 접근 허용 창이
   * 연달아 뜰 수 있어 자동 검사하지 않는다. (버튼으로 직접 실행)
   */
  private autoScanAllowed(path: string): boolean {
    const first = this.listing?.crumbs[0];
    const home = first?.name === "~" ? first.path : null;
    if (home && same(path, home)) return false;
    const underHome = home != null && path.normalize("NFC").startsWith(home.normalize("NFC") + "/");
    return underHome || path.split("/").filter(Boolean).length > 2;
  }

  private cachedScan(path: string): FormScan | null {
    const hit = this.scans.get(path.normalize("NFC"));
    return hit && Date.now() - hit.at < SCAN_TTL_MS ? hit.scan : null;
  }

  private async runScan(path: string, force = false) {
    const key = path.normalize("NFC");
    if (this.scanning.has(key) || (!force && this.cachedScan(path))) return;
    this.scanning.add(key);
    this.renderDetail(true);
    try {
      const scan = await api.scanTree(path);
      this.scans.set(key, { scan, at: Date.now() });
    } catch (e) {
      toast(errMsg(e), "error");
    } finally {
      this.scanning.delete(key);
      this.renderDetail(true);
    }
  }

  private async convert(s: Subject, scope: Scope, btn: HTMLButtonElement) {
    await busy(btn, async () => {
      const rep = await api.convertPaths([s.path], scope, "nfc", false);
      this.scans.clear();
      if (rep.errors.length) toast(`${rep.errors.length}개 항목 변환 실패: ${rep.errors[0].message}`, "error");
      else toast(rep.renamed.length ? `${rep.renamed.length}개 이름을 NFC로 변환했습니다` : "바꿀 이름이 없습니다", "success");
      // 지금 보고 있는 폴더 자신의 이름이 바뀌었으면 새 경로로 이동
      const moved = rep.renamed.find((r) => same(r.from, this.listing?.path ?? ""));
      if (moved) await this.navigate(moved.to, { history: false });
      else await this.refresh();
      await this.app.refreshStatus();
    });
  }

  private onDetailClick(e: Event) {
    const t = e.target as HTMLElement;
    const op = t.closest<HTMLElement>("[data-op]");
    const s = this.subject();
    if (!op || !s) return;
    switch (op.dataset.op) {
      case "convert-name":
        return void this.convert(s, "name", op as HTMLButtonElement);
      case "convert-tree":
        return void this.convert(s, this.scope, op as HTMLButtonElement);
      case "scope":
        this.scope = op.dataset.v as Scope;
        return this.renderDetail(true);
      case "send":
        return this.app.sendToConvert([s.path]);
      case "reveal":
        return void api.revealInFinder(s.path);
      case "copy":
        void navigator.clipboard
          .writeText(s.name.normalize("NFC"))
          .then(() => toast("NFC 이름을 복사했습니다", "success"))
          .catch((err) => toast(errMsg(err), "error"));
        return;
      case "open":
        return void this.navigate(s.path);
      case "rescan":
        return void this.runScan(s.path, true);
      case "more":
        this.samplesShown += SAMPLE_PAGE;
        return this.renderDetail(true);
      case "sample": {
        const p = op.dataset.path!;
        return void this.navigate(parentOf(p), { select: p });
      }
    }
  }

  // ───────── 렌더링 ─────────

  private renderActions() {
    if (!this.actions) return;
    this.actions.innerHTML = `
      <label class="check"><input type="checkbox" id="br-hidden" ${this.showHidden ? "checked" : ""}/> 숨김 파일 표시 <kbd>.</kbd></label>`;
    $("#br-hidden", this.actions).addEventListener("change", () => this.toggleHidden());
  }

  private renderAll() {
    this.renderChrome();
    this.renderList();
    this.renderDetail();
  }

  private renderChrome() {
    const r = this.root;
    const l = this.listing;
    if (!l) return;
    ($("#br-back", r) as HTMLButtonElement).disabled = this.back.length === 0;
    ($("#br-fwd", r) as HTMLButtonElement).disabled = this.forward.length === 0;
    ($("#br-up", r) as HTMLButtonElement).disabled = !l.parent;
    $("#br-crumbs", r).innerHTML = l.crumbs
      .map(
        (c, i) =>
          `${i ? `<span class="crumb-sep">›</span>` : ""}<button class="crumb ${i === l.crumbs.length - 1 ? "current" : ""}" data-path="${esc(c.path)}">${esc(c.name)}</button>`,
      )
      .join("");
    // 긴 경로는 오른쪽(현재 폴더)이 보이도록
    const crumbs = $("#br-crumbs", r);
    crumbs.scrollLeft = crumbs.scrollWidth;
  }

  private renderList() {
    const r = this.root;
    const l = this.listing;
    if (!l) return;

    const counts = { all: l.items.length, nfd: 0, nfc: 0, ascii: 0 };
    for (const i of l.items) {
      if (needsFix(i.form)) counts.nfd++;
      else if (i.form === "nfc") counts.nfc++;
      else counts.ascii++;
    }
    r.querySelectorAll<HTMLElement>("#br-filter button").forEach((b) => {
      b.classList.toggle("active", b.dataset.v === this.filter);
      const c = b.querySelector<HTMLElement>("[data-count]");
      if (c) c.textContent = String(counts[c.dataset.count as keyof typeof counts]);
      b.classList.toggle("has-warn", b.dataset.v === "nfd" && counts.nfd > 0);
    });

    const items = this.visibleItems();
    const empty =
      l.items.length === 0
        ? "빈 폴더입니다"
        : items.length === 0
          ? "조건에 맞는 항목이 없습니다"
          : "";
    $("#br-list", r).innerHTML =
      (empty ? `<li class="muted empty-row">${empty}</li>` : "") +
      items
        .map((it) => {
          const sel = this.selected != null && same(it.path, this.selected);
          return `
        <li data-path="${esc(it.path)}" class="${sel ? "selected" : ""} ${it.isDir ? "dir" : ""} ${needsFix(it.form) ? "warn" : ""}">
          <span class="fi">${it.isDir ? "📁" : "📄"}</span>
          <span class="name" title="${esc(it.name)}">${esc(it.name)}${it.isSymlink ? ` <span class="muted">↪</span>` : ""}</span>
          <span>${formBadge(it.form, true)}</span>
          <span class="r muted">${it.size != null ? formatBytes(it.size) : ""}</span>
          <span class="muted">${formatDate(it.modified)}</span>
          <span class="r">${
            it.isDir
              ? `<button class="icon-btn info" data-op="info" title="들어가지 않고 정보 보기">ⓘ</button><span class="chev">›</span>`
              : ""
          }</span>
        </li>`;
        })
        .join("");
  }

  /** @param force 같은 대상이어도 다시 그린다 (검사 결과 도착 등) */
  private renderDetail(force = false) {
    const el = $("#br-detail", this.root);
    const s = this.subject();
    if (!s) {
      el.innerHTML = "";
      return;
    }
    const key = `${s.path}|${s.rawName}|${s.form}|${s.size}|${s.modified}`;
    if (!force && key === this.detailKey) return;
    this.detailKey = key;

    const nfcName = s.name.normalize("NFC");
    const broken = needsFix(s.form);
    const groups = analyzeName(s.rawName);
    const nonAscii = groups.filter((g) => !/^[\x00-\x7F]$/.test(g.stored));
    const decomposed = groups.filter((g) => g.decomposed).length;

    const header = `
      <div class="d-head">
        <div class="d-icon">${s.isDir ? "📁" : "📄"}</div>
        <div class="d-title">
          <div class="d-name" title="${esc(nfcName)}">${esc(nfcName)}</div>
          <div class="d-sub">${s.isCurrent ? "현재 폴더" : s.isDir ? "폴더" : "파일"} · ${formBadge(s.form, true)} <span class="muted">${FORM_LABEL[s.form]}</span></div>
        </div>
      </div>`;

    const windowsBox =
      s.form === "ascii"
        ? ""
        : `<div class="d-section">
        <div class="d-label">Windows·zip에서 보이는 이름</div>
        <div class="win-name ${broken ? "bad" : "good"}">${esc(broken ? visualizeNfd(s.rawName) : nfcName)}</div>
        ${
          broken
            ? `<div class="d-label">NFC로 변환하면</div><div class="win-name good">${esc(nfcName)}</div>`
            : `<p class="muted small">✓ 완성형으로 저장되어 있어 Windows에서도 그대로 보입니다.</p>`
        }
      </div>`;

    const analysis =
      nonAscii.length === 0
        ? ""
        : `<div class="d-section">
        <div class="d-label">글자별 저장 형태 <span class="muted">(한글 등 ${nonAscii.length}글자${decomposed ? ` 중 <b class="accent">${decomposed}글자 분해됨</b>` : ""})</span></div>
        <table class="cp-table">
          ${nonAscii
            .slice(0, 40)
            .map(
              (g) => `<tr class="${g.decomposed ? "dec" : ""}">
                <td class="glyph">${esc(g.glyph)}</td>
                <td class="cps">${g.codepoints.map((c) => `<code>${c}</code>`).join(" ")}</td>
                <td class="st">${g.decomposed ? `<span class="status warn">분해 ${g.codepoints.length}개</span>` : `<span class="status ok">완성형</span>`}</td>
              </tr>`,
            )
            .join("")}
        </table>
        ${nonAscii.length > 40 ? `<p class="muted small">… 외 ${nonAscii.length - 40}글자</p>` : ""}
        <div class="kv">
          <span>코드 포인트</span><b>${Array.from(s.rawName).length}${broken ? ` → ${Array.from(nfcName).length}` : ""}</b>
          <span>UTF-8 바이트</span><b>${utf8Len(s.rawName)}${broken ? ` → ${utf8Len(nfcName)}` : ""}</b>
          ${s.size != null ? `<span>크기</span><b>${formatBytes(s.size)}</b>` : ""}
          ${s.modified != null ? `<span>수정일</span><b>${formatDate(s.modified)}</b>` : ""}
        </div>
      </div>`;

    const plainMeta =
      nonAscii.length === 0 && (s.size != null || s.modified != null)
        ? `<div class="d-section"><div class="kv">
            ${s.size != null ? `<span>크기</span><b>${formatBytes(s.size)}</b>` : ""}
            ${s.modified != null ? `<span>수정일</span><b>${formatDate(s.modified)}</b>` : ""}
          </div></div>`
        : "";

    const tree = s.isDir ? this.treeSection(s) : "";

    const actions = `
      <div class="d-section d-actions">
        ${
          s.isDir
            ? `<div class="segmented small">
                ${(
                  [
                    ["recursive", "하위 전체"],
                    ["children", "바로 아래만"],
                    ["name", "이름만"],
                  ] as const
                )
                  .map(
                    ([v, label]) =>
                      `<button data-op="scope" data-v="${v}" class="${this.scope === v ? "active" : ""}">${label}</button>`,
                  )
                  .join("")}
              </div>
              <button class="btn primary" data-op="convert-tree">NFC로 변환</button>`
            : `<button class="btn primary" data-op="convert-name" ${broken ? "" : "disabled"}>NFC로 변환</button>`
        }
        <div class="row gap wrap">
          ${s.isDir && !s.isCurrent ? `<button class="btn" data-op="open">열기</button>` : ""}
          <button class="btn" data-op="send">변환 목록으로</button>
          <button class="btn" data-op="reveal">Finder에서 보기</button>
          ${s.form !== "ascii" ? `<button class="btn" data-op="copy">NFC 이름 복사</button>` : ""}
        </div>
      </div>`;

    const prevScroll = el.scrollTop;
    el.innerHTML = header + windowsBox + analysis + plainMeta + tree + actions;
    if (force) el.scrollTop = prevScroll;

    if (s.isDir && this.autoScanAllowed(s.path)) void this.runScan(s.path);
  }

  private treeSection(s: Subject): string {
    const scan = this.cachedScan(s.path);
    const busyNow = this.scanning.has(s.path.normalize("NFC"));
    if (!scan) {
      const body = busyNow
        ? `<p class="muted small"><span class="spinner"></span> 하위 항목을 검사하는 중…</p>`
        : this.autoScanAllowed(s.path)
          ? `<p class="muted small">검사 대기 중</p>`
          : `<p class="muted small">홈·최상위 폴더는 항목이 많아 자동으로 검사하지 않습니다.</p>
             <button class="btn" data-op="rescan">하위 전체 검사 실행</button>`;
      return `<div class="d-section"><div class="d-label">하위 전체 검사</div>${body}</div>`;
    }
    const fix = scan.nfd + scan.mixed;
    const pct = (n: number) => (scan.scanned ? (n / scan.scanned) * 100 : 0);
    const samples = scan.samples.slice(0, this.samplesShown);
    return `
      <div class="d-section">
        <div class="d-label">하위 전체 검사
          <button class="btn link small" data-op="rescan" ${busyNow ? "disabled" : ""}>${busyNow ? "검사 중…" : "다시 검사"}</button>
        </div>
        <div class="form-bar" title="NFD ${scan.nfd} · 혼합 ${scan.mixed} · NFC ${scan.nfc} · ASCII ${scan.ascii}">
          <span class="seg nfd" style="width:${pct(scan.nfd)}%"></span>
          <span class="seg mixed" style="width:${pct(scan.mixed)}%"></span>
          <span class="seg nfc" style="width:${pct(scan.nfc)}%"></span>
          <span class="seg ascii" style="width:${pct(scan.ascii)}%"></span>
        </div>
        <div class="legend">
          <span><i class="nfd"></i>NFD ${scan.nfd.toLocaleString()}</span>
          <span><i class="mixed"></i>혼합 ${scan.mixed.toLocaleString()}</span>
          <span><i class="nfc"></i>NFC ${scan.nfc.toLocaleString()}</span>
          <span><i class="ascii"></i>ASCII ${scan.ascii.toLocaleString()}</span>
        </div>
        <p class="small ${fix ? "" : "muted"}">${scan.scanned.toLocaleString()}개 검사${scan.truncated ? " (상한 도달, 일부만 검사)" : ""} ·
          ${fix ? `<b class="accent">변환 필요 ${fix.toLocaleString()}개</b>` : "모두 정상입니다 ✓"}</p>
        ${
          samples.length
            ? `<ul class="sample-list">
                ${samples
                  .map(
                    (x) => `<li data-op="sample" data-path="${esc(x.path)}" title="이 위치로 이동">
                      <span class="fi">${x.isDir ? "📁" : "📄"}</span>
                      <span class="name">${esc(x.relative)}</span>${formBadge(x.form)}</li>`,
                  )
                  .join("")}
              </ul>
              ${
                scan.samples.length > samples.length
                  ? `<button class="btn link small" data-op="more">더 보기 (${scan.samples.length - samples.length}개)</button>`
                  : fix > scan.samples.length
                    ? `<p class="muted small">표본 ${scan.samples.length}개만 표시합니다.</p>`
                    : ""
              }`
            : ""
        }
      </div>`;
  }
}
