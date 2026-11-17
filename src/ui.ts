// 작은 DOM 유틸리티 — 프레임워크 없이 쓰기 위한 최소한의 도구들
import type { UnicodeForm } from "./api";

const ESC: Record<string, string> = {
  "&": "&amp;",
  "<": "&lt;",
  ">": "&gt;",
  '"': "&quot;",
  "'": "&#39;",
};

/** HTML 이스케이프. 파일명은 사용자 데이터이므로 템플릿에 넣을 때 반드시 거친다. */
export function esc(s: unknown): string {
  return String(s ?? "").replace(/[&<>"']/g, (c) => ESC[c]);
}

export function $(sel: string, root: ParentNode = document): HTMLElement {
  const el = root.querySelector<HTMLElement>(sel);
  if (!el) throw new Error(`element not found: ${sel}`);
  return el;
}

export function $$(sel: string, root: ParentNode = document): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(sel));
}

export function formBadge(form: UnicodeForm): string {
  switch (form) {
    case "nfd":
      return `<span class="badge badge-nfd" title="자소 분리형 — Windows에서 깨져 보입니다">NFD</span>`;
    case "mixed":
      return `<span class="badge badge-mixed" title="조합형과 분해형이 섞여 있습니다">혼합</span>`;
    case "nfc":
      return `<span class="badge badge-nfc" title="완성형 — Windows 호환">NFC</span>`;
    default:
      return "";
  }
}

export function basename(p: string): string {
  const parts = p.split("/").filter(Boolean);
  return parts[parts.length - 1] ?? p;
}

/** 표시용으로 NFC 정규화 (WKWebView는 NFD도 붙여서 그려 주지만 비교·검색을 위해) */
export function nfc(s: string): string {
  return s.normalize("NFC");
}

export function formatBytes(n: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  let v = n;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}

export function errMsg(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return JSON.stringify(e);
}

// ───────── 토스트 ─────────

export type ToastKind = "success" | "error" | "info";

export function toast(message: string, kind: ToastKind = "info") {
  const host = $("#toasts");
  const el = document.createElement("div");
  el.className = `toast toast-${kind}`;
  el.innerHTML = `<span class="toast-ico">${kind === "success" ? "✓" : kind === "error" ? "!" : "i"}</span><span>${esc(message)}</span>`;
  host.appendChild(el);
  while (host.children.length > 5) host.firstElementChild?.remove();
  // 원본 TUI와 동일: 성공 3초, 오류 6초
  const ttl = kind === "error" ? 6000 : 3000;
  setTimeout(() => {
    el.classList.add("leaving");
    setTimeout(() => el.remove(), 250);
  }, ttl);
}

/** 버튼을 누르는 동안 비활성화하고 스피너를 보여준다. */
export async function busy<T>(btn: HTMLButtonElement, fn: () => Promise<T>): Promise<T | undefined> {
  if (btn.disabled) return;
  btn.disabled = true;
  btn.classList.add("is-busy");
  try {
    return await fn();
  } catch (e) {
    toast(errMsg(e), "error");
    return undefined;
  } finally {
    btn.disabled = false;
    btn.classList.remove("is-busy");
  }
}
