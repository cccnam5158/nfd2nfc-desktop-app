// 변환 · 압축 탭 — 끌어다 놓은 파일/폴더를 그 자리에서 NFC로 바꾸거나, Windows용 NFC zip을 만든다.
import { open, save } from "@tauri-apps/plugin-dialog";

import { api, type AnalyzedItem, type Report, type Scope, type Target, type ZipReport } from "../api";
import type { App, Tab } from "../main";
import {
  $,
  basename,
  busy,
  errMsg,
  esc,
  formBadge,
  formatBytes,
  toast,
  visualizeNfd,
} from "../ui";

const MAX_RESULT_ROWS = 300;

export class ConvertTab implements Tab {
  readonly title = "변환 · 압축";
  readonly dropHint = "놓으면 변환 목록에 추가합니다";
  private root!: HTMLElement;
  private items: AnalyzedItem[] = [];
  private scope: Scope = "recursive";
  private target: Target = "nfc";
  private zipOutput = "";
  private zipOutputTouched = false;
  private analyzing = 0;

  constructor(private app: App) {}

  mount(root: HTMLElement) {
    this.root = root;
    root.innerHTML = `
      <div class="convert">
        <div class="dropzone" id="cv-drop">
          <div class="dropzone-ico">⇣</div>
          <div class="dropzone-text">
            <b>파일이나 폴더를 여기로 끌어다 놓으세요</b>
            <span>Windows로 보내기 전에 이름을 고치거나, 바로 Windows용 zip을 만들 수 있습니다.</span>
          </div>
          <div class="row gap">
            <button class="btn" id="cv-pick-files">파일 선택…</button>
            <button class="btn" id="cv-pick-dirs">폴더 선택…</button>
          </div>
        </div>

        <div class="queue" id="cv-queue" hidden>
          <div class="queue-head">
            <span id="cv-summary"></span>
            <button class="btn link" id="cv-clear">목록 비우기</button>
          </div>
          <ul class="queue-list" id="cv-list"></ul>
        </div>

        <div class="panels" id="cv-panels" hidden>
          <div class="card panel">
            <div class="card-title">그 자리에서 이름 바꾸기</div>
            <div class="field">
              <label>범위</label>
              <div class="segmented" id="cv-scope">
                <button data-v="recursive">전체 (하위 포함)</button>
                <button data-v="children">바로 아래만</button>
                <button data-v="name">이름만</button>
              </div>
            </div>
            <div class="field">
              <label>방향</label>
              <div class="segmented" id="cv-target">
                <button data-v="nfc">NFD → NFC</button>
                <button data-v="nfd">NFC → NFD (역변환)</button>
              </div>
            </div>
            <div class="row gap end">
              <button class="btn" id="cv-preview">미리보기</button>
              <button class="btn primary" id="cv-run">변환 실행</button>
            </div>
          </div>

          <div class="card panel">
            <div class="card-title">Windows용 zip 만들기</div>
            <p class="muted small">원본은 그대로 두고, zip 안의 파일명만 NFC로 넣습니다. 함께 UTF-8 플래그를 표시하고
              <code>.DS_Store</code>·<code>._*</code> 같은 macOS 전용 파일은 빼서 Windows 탐색기에서 깔끔하게 풀리게 합니다.</p>
            <div class="field">
              <label>저장 위치</label>
              <div class="path-input">
                <input type="text" id="cv-zip-out" spellcheck="false" />
                <button class="btn" id="cv-zip-browse">변경…</button>
              </div>
            </div>
            <label class="check"><input type="checkbox" id="cv-zip-hidden" /> 숨김 파일(.으로 시작)도 포함</label>
            <div class="row gap end">
              <button class="btn primary" id="cv-zip">zip 만들기</button>
            </div>
          </div>
        </div>

        <div class="result" id="cv-result" hidden></div>
      </div>`;

    $("#cv-pick-files", root).addEventListener("click", () => void this.pick(false));
    $("#cv-pick-dirs", root).addEventListener("click", () => void this.pick(true));
    $("#cv-clear", root).addEventListener("click", () => {
      this.items = [];
      this.zipOutputTouched = false;
      $("#cv-result", root).hidden = true;
      this.render();
    });
    $("#cv-list", root).addEventListener("click", (e) => {
      const t = e.target as HTMLElement;
      const li = t.closest<HTMLElement>("li[data-i]");
      if (!li) return;
      const i = Number(li.dataset.i);
      const op = t.closest<HTMLElement>("[data-op]")?.dataset.op;
      if (op === "remove") {
        this.items.splice(i, 1);
        this.zipOutputTouched = false;
        this.render();
      } else if (op === "reveal") {
        void api.revealInFinder(this.items[i].path);
      }
    });

    this.segmented("#cv-scope", () => this.scope, (v) => (this.scope = v as Scope));
    this.segmented("#cv-target", () => this.target, (v) => (this.target = v as Target));

    const out = $("#cv-zip-out", root) as HTMLInputElement;
    out.addEventListener("input", () => {
      this.zipOutput = out.value;
      this.zipOutputTouched = true;
    });
    $("#cv-zip-browse", root).addEventListener("click", async () => {
      const p = await save({
        title: "zip 저장 위치",
        defaultPath: this.zipOutput || undefined,
        filters: [{ name: "ZIP", extensions: ["zip"] }],
      });
      if (p) {
        this.zipOutput = p;
        this.zipOutputTouched = true;
        out.value = p;
      }
    });

    const preview = $("#cv-preview", root) as HTMLButtonElement;
    preview.addEventListener("click", () =>
      busy(preview, async () => {
        const rep = await api.convertPaths(this.paths(), this.scope, this.target, true);
        this.showReport(rep, true);
      }),
    );
    const run = $("#cv-run", root) as HTMLButtonElement;
    run.addEventListener("click", () =>
      busy(run, async () => {
        const rep = await api.convertPaths(this.paths(), this.scope, this.target, false);
        this.showReport(rep, false);
        if (rep.errors.length) toast(`${rep.errors.length}개 항목을 변환하지 못했습니다`, "error");
        else toast(rep.renamed.length ? `${rep.renamed.length}개 이름을 변환했습니다` : "바꿀 이름이 없습니다", "success");
        // 경로가 바뀌었으니 변환된 최상위 항목 경로를 갱신하고 다시 분석
        const moved = new Map(rep.renamed.map((r) => [r.from, r.to]));
        await this.reanalyze(this.paths().map((p) => moved.get(p) ?? p));
        await this.app.refreshStatus();
      }),
    );
    const zip = $("#cv-zip", root) as HTMLButtonElement;
    zip.addEventListener("click", () =>
      busy(zip, async () => {
        const output = (this.zipOutput || "").trim();
        if (!output) throw new Error("저장 위치를 지정하세요");
        const hidden = ($("#cv-zip-hidden", root) as HTMLInputElement).checked;
        const rep = await api.createZip(this.paths(), output, hidden);
        this.showZip(rep);
        toast(`zip을 만들었습니다 — ${basename(rep.output)}`, "success");
        this.zipOutputTouched = false;
        await this.updateZipOutput();
      }),
    );

    this.render();
  }

  onDrop(paths: string[]) {
    void this.add(paths);
  }

  private paths() {
    return this.items.map((i) => i.path);
  }

  private segmented(sel: string, get: () => string, set: (v: string) => void) {
    const el = $(sel, this.root);
    const sync = () =>
      el.querySelectorAll<HTMLButtonElement>("button").forEach((b) =>
        b.classList.toggle("active", b.dataset.v === get()),
      );
    el.addEventListener("click", (e) => {
      const b = (e.target as HTMLElement).closest<HTMLButtonElement>("button[data-v]");
      if (!b) return;
      set(b.dataset.v!);
      sync();
    });
    sync();
  }

  private async pick(directory: boolean) {
    const picked = await open({
      directory,
      multiple: true,
      title: directory ? "변환할 폴더 선택" : "변환할 파일 선택",
    });
    if (!picked) return;
    await this.add(Array.isArray(picked) ? picked : [picked]);
  }

  private async add(paths: string[]) {
    const known = new Set(this.items.map((i) => i.path.normalize("NFC")));
    const fresh = paths.filter((p) => !known.has(p.normalize("NFC")));
    if (!fresh.length) return;
    this.analyzing++;
    this.render();
    try {
      const analyzed = await api.analyzePaths(fresh);
      this.items.push(...analyzed);
      $("#cv-result", this.root).hidden = true;
    } catch (e) {
      toast(errMsg(e), "error");
    } finally {
      this.analyzing--;
      this.render();
      await this.updateZipOutput();
    }
  }

  private async reanalyze(paths: string[]) {
    this.items = await api.analyzePaths(paths);
    this.render();
    await this.updateZipOutput();
  }

  private async updateZipOutput() {
    if (this.zipOutputTouched || !this.items.length) return;
    this.zipOutput = (await api.suggestZipPath(this.paths())) ?? "";
    ($("#cv-zip-out", this.root) as HTMLInputElement).value = this.zipOutput;
  }

  private render() {
    const r = this.root;
    const has = this.items.length > 0;
    $("#cv-queue", r).hidden = !has && !this.analyzing;
    $("#cv-panels", r).hidden = !has;
    $("#cv-drop", r).classList.toggle("compact", has);

    const totalNfd = this.items.reduce((s, i) => s + i.nfdCount, 0);
    $("#cv-summary", r).innerHTML = this.analyzing
      ? `<span class="spinner"></span> 분석 중…`
      : `항목 <b>${this.items.length}</b>개 · 변환이 필요한 이름 <b class="${totalNfd ? "accent" : ""}">${totalNfd.toLocaleString()}</b>개`;

    $("#cv-list", r).innerHTML = this.items
      .map(
        (it, i) => `
        <li data-i="${i}">
          <span class="fi">${it.isDir ? "📁" : "📄"}</span>
          <span class="name" title="${esc(it.display)}">${esc(it.name)}</span>
          ${formBadge(it.form)}
          <span class="meta">${
            it.error
              ? `<span class="status err">${esc(it.error)}</span>`
              : it.nfdCount
                ? `<span class="status warn">NFD ${it.nfdCount.toLocaleString()}개</span>`
                : `<span class="status ok">정상</span>`
          }${it.isDir ? ` <span class="muted">/ ${it.scanned.toLocaleString()}개 검사</span>` : ""}</span>
          <button class="icon-btn" data-op="reveal" title="Finder에서 보기">⌕</button>
          <button class="icon-btn danger" data-op="remove" title="목록에서 빼기">✕</button>
        </li>`,
      )
      .join("");
  }

  private showReport(rep: Report, dryRun: boolean) {
    const el = $("#cv-result", this.root);
    el.hidden = false;
    const rows = rep.renamed.slice(0, MAX_RESULT_ROWS);
    const more = rep.renamed.length - rows.length;
    el.innerHTML = `
      <div class="result-head">
        <b>${dryRun ? "미리보기" : "변환 결과"}</b>
        <span>${rep.scanned.toLocaleString()}개 검사 · ${rep.renamed.length.toLocaleString()}개 ${dryRun ? "변경 예정" : "변경됨"}${
          rep.errors.length ? ` · <span class="status err">오류 ${rep.errors.length}개</span>` : ""
        }</span>
      </div>
      ${
        rep.renamed.length === 0 && rep.errors.length === 0
          ? `<p class="muted">바꿀 이름이 없습니다. 모두 ${this.target === "nfc" ? "NFC" : "NFD"} 형식입니다.</p>`
          : ""
      }
      <ul class="rename-list">
        ${rows
          .map(
            (r) => `<li><span class="fi">${r.isDir ? "📁" : "📄"}</span>
              <span class="from">${esc(visualizeNfd(basename(r.from)))}</span>
              <span class="arrow">→</span>
              <span class="to">${esc(basename(r.to).normalize("NFC"))}</span>
              <span class="muted dir">${esc(parentDisplay(r.to))}</span></li>`,
          )
          .join("")}
        ${more > 0 ? `<li class="muted">… 외 ${more.toLocaleString()}개</li>` : ""}
        ${rep.errors
          .slice(0, 100)
          .map(
            (e) =>
              `<li class="err"><span class="fi">!</span><span class="from">${esc(basename(e.path).normalize("NFC"))}</span><span class="muted">${esc(e.message)}</span></li>`,
          )
          .join("")}
      </ul>`;
  }

  private showZip(rep: ZipReport) {
    const el = $("#cv-result", this.root);
    el.hidden = false;
    el.innerHTML = `
      <div class="result-head"><b>zip 생성 완료</b>
        <button class="btn link" id="cv-zip-reveal">Finder에서 보기</button></div>
      <p><code>${esc(rep.output.normalize("NFC"))}</code></p>
      <p class="muted">파일 ${rep.files.toLocaleString()}개 · 폴더 ${rep.dirs.toLocaleString()}개 ·
        원본 ${formatBytes(rep.bytes)} · NFC로 바꿔 넣은 이름 <b>${rep.normalizedNames.toLocaleString()}</b>개 ·
        제외 ${rep.skipped.toLocaleString()}개</p>`;
    $("#cv-zip-reveal", el).addEventListener("click", () => void api.revealInFinder(rep.output));
  }
}

function parentDisplay(p: string): string {
  const parts = p.split("/");
  parts.pop();
  return parts.slice(-2).join("/").normalize("NFC");
}
