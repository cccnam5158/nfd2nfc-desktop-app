import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";

import { api, type AppStatus } from "../api";
import type { App, Tab } from "../main";
import { $, busy, esc, toast } from "../ui";

export class HomeTab implements Tab {
  readonly title = "홈";
  private root!: HTMLElement;

  constructor(private app: App) {}

  mount(root: HTMLElement) {
    this.root = root;
    root.innerHTML = `
      <div class="home">
        <div class="hero">
          <div class="hero-text">
            <h2>한글 파일명, 이제 Windows에서도 그대로.</h2>
            <p>macOS는 한글 파일명을 자소 단위로 분해(NFD)해 저장합니다. 이런 파일을 Windows로 보내면
               <b>ㅎㅏㄴㄱㅡㄹ</b>처럼 보입니다. NFD2NFC는 지정한 폴더를 지켜보다가 이런 파일명을
               완성형(NFC)으로 자동으로 바꿉니다.</p>
          </div>
          <div class="example">
            <div class="example-row bad"><span class="tag">Windows에서 보이는 이름</span><code>ㅇㅣㄹㅅㅏㅇㄱㅣㄹㅗㄱ.txt</code></div>
            <div class="example-arrow">↓</div>
            <div class="example-row good"><span class="tag">변환 후</span><code>일상기록.txt</code></div>
          </div>
        </div>

        <div class="cards">
          <div class="card status-card">
            <div class="status-big" id="home-status">
              <span class="pulse"></span>
              <div>
                <div class="status-title" id="home-status-title">확인 중…</div>
                <div class="status-sub" id="home-status-sub"></div>
              </div>
            </div>
            <div class="row gap">
              <button class="btn primary" id="home-toggle">감시 시작</button>
              <button class="btn" id="home-restart">재시작</button>
            </div>
          </div>

          <div class="card metric-card">
            <div class="card-title">이번 세션</div>
            <div class="metric"><span id="home-converted">0</span><small>개 이름 변환</small></div>
            <button class="btn link" id="home-go-logs">로그 보기 →</button>
          </div>

          <div class="card folders-card">
            <div class="card-title">감시 폴더 <span class="muted" id="home-folder-count"></span></div>
            <ul class="folder-mini" id="home-folders"></ul>
            <button class="btn link" id="home-go-config">관리 →</button>
          </div>
        </div>

        <div class="card settings">
          <div class="setting">
            <div>
              <div class="setting-title">로그인 시 자동 실행</div>
              <div class="setting-desc">Mac에 로그인하면 메뉴바에서 조용히 시작해 감시를 이어 갑니다.</div>
            </div>
            <label class="switch"><input type="checkbox" id="home-autostart" /><span></span></label>
          </div>
          <div class="setting">
            <div>
              <div class="setting-title">전체 디스크 접근 권한 <span id="home-fda-badge"></span></div>
              <div class="setting-desc">권장 사항입니다. 허용하지 않으면 데스크탑·문서·다운로드 폴더마다 접근 허용 창이 뜨고,
                일부 위치(iCloud Drive, 외장 디스크 등)는 감시하지 못할 수 있습니다.</div>
            </div>
            <button class="btn" id="home-fda">시스템 설정 열기</button>
          </div>
          <div class="setting">
            <div>
              <div class="setting-title">창을 닫아도 계속 실행됩니다</div>
              <div class="setting-desc">메뉴바 아이콘에서 감시를 켜고 끄거나 앱을 종료할 수 있습니다.</div>
            </div>
          </div>
        </div>

        <footer class="home-footer">
          <span id="home-version"></span>
          <span>설정 <code id="home-config-path"></code></span>
          <span>참고: elgar328/nfd2nfc (MIT)</span>
        </footer>
      </div>`;

    const toggle = $("#home-toggle", root) as HTMLButtonElement;
    toggle.addEventListener("click", () =>
      busy(toggle, async () => {
        const running = this.app.status?.watcher.running ?? false;
        const w = await api.setWatching(!running);
        this.app.patchWatcher(w);
        toast(w.running ? "감시를 시작했습니다" : "감시를 중지했습니다", "success");
      }),
    );
    const restart = $("#home-restart", root) as HTMLButtonElement;
    restart.addEventListener("click", () =>
      busy(restart, async () => {
        this.app.patchWatcher(await api.restartWatcher());
        toast("감시를 다시 시작했습니다", "success");
      }),
    );
    $("#home-go-logs", root).addEventListener("click", () => this.app.switchTo("logs"));
    $("#home-go-config", root).addEventListener("click", () => this.app.switchTo("config"));
    $("#home-fda", root).addEventListener("click", () => void api.openFdaSettings());

    const auto = $("#home-autostart", root) as HTMLInputElement;
    void isEnabled()
      .then((v) => (auto.checked = v))
      .catch(() => (auto.disabled = true));
    auto.addEventListener("change", async () => {
      try {
        if (auto.checked) await enable();
        else await disable();
        toast(auto.checked ? "로그인 시 자동 실행을 켰습니다" : "자동 실행을 껐습니다", "success");
      } catch (e) {
        auto.checked = !auto.checked;
        toast(String(e), "error");
      }
    });

    this.app.onStatus((s) => this.render(s));
  }

  show() {
    void this.renderFolders();
  }

  /** 감시 폴더 요약 카드 — 실제로 적용 중인 규칙만 보여 준다 */
  private async renderFolders() {
    try {
      const views = await api.getConfig();
      const watching = views.filter((v) => v.action === "watch" && v.status.kind === "active");
      const problems = views.filter((v) => !["active", "redundant"].includes(v.status.kind)).length;
      $("#home-folder-count", this.root).textContent = `${watching.length}개`;
      $("#home-folders", this.root).innerHTML = watching.length
        ? watching
            .slice(0, 5)
            .map(
              (v) =>
                `<li title="${esc(v.display)}"><span class="fi">📁</span><span class="name">${esc(v.display.normalize("NFC"))}</span><span class="muted small">${v.mode === "recursive" ? "하위 전체" : "바로 아래"}</span></li>`,
            )
            .join("") +
          (watching.length > 5 ? `<li class="muted small">… 외 ${watching.length - 5}개</li>` : "") +
          (problems ? `<li class="small"><span class="status err">✗ 문제 있는 규칙 ${problems}개</span></li>` : "")
        : `<li class="muted small">아직 감시 중인 폴더가 없습니다. 파일을 주고받는 폴더(다운로드 등)를 추가하세요.</li>`;
    } catch {
      /* 홈 카드는 보조 정보 — 실패해도 조용히 */
    }
  }

  private render(s: AppStatus) {
    const r = this.root;
    const running = s.watcher.running;
    $("#home-status", r).classList.toggle("running", running);
    $("#home-status-title", r).textContent = running ? "감시 중" : "중지됨";
    $("#home-status-sub", r).textContent = running
      ? s.watcher.watched > 0
        ? `폴더 ${s.watcher.watched}개를 실시간으로 지켜보고 있습니다`
        : "감시할 폴더가 없습니다 — ‘감시 폴더’에서 추가하세요"
      : "새로 생기는 NFD 파일명이 변환되지 않습니다";
    const toggle = $("#home-toggle", r);
    toggle.textContent = running ? "감시 중지" : "감시 시작";
    toggle.classList.toggle("primary", !running);
    ($("#home-restart", r) as HTMLButtonElement).disabled = !running;
    $("#home-converted", r).textContent = s.sessionConverted.toLocaleString();
    $("#home-version", r).textContent = `v${s.version}`;
    $("#home-config-path", r).textContent = s.configPath;

    const fda = s.fullDiskAccess;
    $("#home-fda-badge", r).innerHTML =
      fda === true
        ? `<span class="pill ok">허용됨</span>`
        : fda === false
          ? `<span class="pill warn">허용 안 됨</span>`
          : `<span class="pill muted">${esc("알 수 없음")}</span>`;
  }
}
