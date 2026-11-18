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

export function formBadge(form: UnicodeForm, showAscii = false): string {
  switch (form) {
    case "nfd":
      return `<span class="badge badge-nfd" title="자소 분리형 — Windows에서 깨져 보입니다">NFD</span>`;
    case "mixed":
      return `<span class="badge badge-mixed" title="조합형과 분해형이 섞여 있습니다">혼합</span>`;
    case "nfc":
      return `<span class="badge badge-nfc" title="완성형 — Windows 호환">NFC</span>`;
    default:
      return showAscii
        ? `<span class="badge badge-ascii" title="영문·숫자만 — 정규화와 무관">ASCII</span>`
        : "";
  }
}

export const FORM_LABEL: Record<UnicodeForm, string> = {
  nfd: "NFD (자소 분리)",
  mixed: "혼합 (일부 분리)",
  nfc: "NFC (완성형)",
  ascii: "ASCII",
};

// ───────── 유니코드 분석 ─────────

/**
 * NFD 문자열을 "Windows에서 보이는 모습"으로 시각화한다.
 * 조합용 자모(U+1100~)를 호환 자모(ㄱ, ㅏ…)로 바꿔 자소 분리가 눈에 보이게 한다.
 */
export function visualizeNfd(s: string): string {
  const CHO = "ㄱㄲㄴㄷㄸㄹㅁㅂㅃㅅㅆㅇㅈㅉㅊㅋㅌㅍㅎ";
  const JUNG = "ㅏㅐㅑㅒㅓㅔㅕㅖㅗㅘㅙㅚㅛㅜㅝㅞㅟㅠㅡㅢㅣ";
  const JONG = " ㄱㄲㄳㄴㄵㄶㄷㄹㄺㄻㄼㄽㄾㄿㅀㅁㅂㅄㅅㅆㅇㅈㅊㅋㅌㅍㅎ";
  let out = "";
  for (const ch of s) {
    const c = ch.codePointAt(0)!;
    if (c >= 0x1100 && c <= 0x1112) out += CHO[c - 0x1100];
    else if (c >= 0x1161 && c <= 0x1175) out += JUNG[c - 0x1161];
    else if (c >= 0x11a8 && c <= 0x11c2) out += JONG[c - 0x11a7];
    else out += ch;
  }
  return out;
}

/** 한글 중성·종성(조합용 자모)과 일반 결합 문자는 앞 글자에 붙는다. */
const COMBINING = /[ᅠ-ᇿힰ-퟿]|\p{M}/u;

export interface CharGroup {
  /** 디스크에 저장된 그대로 */
  stored: string;
  /** 사람이 보는 한 글자 (NFC) */
  glyph: string;
  codepoints: string[];
  /** 저장 형태가 NFC와 다름 = 분해되어 있음 */
  decomposed: boolean;
}

export function codepoint(ch: string): string {
  return "U+" + ch.codePointAt(0)!.toString(16).toUpperCase().padStart(4, "0");
}

/** 원본 이름을 "보이는 한 글자" 단위로 묶어 각 글자가 어떻게 저장돼 있는지 분석한다. */
export function analyzeName(raw: string): CharGroup[] {
  const groups: string[] = [];
  for (const ch of raw) {
    if (groups.length && COMBINING.test(ch)) groups[groups.length - 1] += ch;
    else groups.push(ch);
  }
  return groups.map((g) => {
    const glyph = g.normalize("NFC");
    return { stored: g, glyph, codepoints: Array.from(g).map(codepoint), decomposed: glyph !== g };
  });
}

const encoder = new TextEncoder();
export function utf8Len(s: string): number {
  return encoder.encode(s).length;
}

export function formatDate(sec: number | null): string {
  if (sec == null) return "—";
  return new Date(sec * 1000).toLocaleString("ko-KR", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
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
