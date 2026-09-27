/**
 * Web 閲覧ページ向けの `/api/events` (SSE) クライアント(#341)。
 *
 * `EventSource` は1本だけ遅延生成して全ページ・全ハンドラで共有する
 * (ページごとに張ると Cookie 認証済みの接続を無駄に増やすだけなので)。
 * 再接続で `open` した直後は、切断中に取りこぼしたかもしれない変更を
 * 拾わせるため合成の `resync` イベントを配る。
 */

export interface ViewerSseEvent {
  type: "artifact-changed" | "state-changed" | "resync";
  scope?: "worktree" | "repository";
  scopeId?: string;
  repoKey?: string;
  artifactId?: string;
  command?: string;
}

type Handler = (event: ViewerSseEvent) => void;

let source: EventSource | null = null;
const handlers = new Set<Handler>();
/** 直近で `open` を経ている接続からの最初の1件か(再接続直後の resync 判定用) */
let sawOpenSinceLastMessage = false;

function ensureSource(): EventSource {
  if (source) return source;
  const es = new EventSource("/api/events");
  es.onopen = () => {
    sawOpenSinceLastMessage = true;
  };
  es.onmessage = (ev) => {
    // 再接続直後の最初のメッセージより先に resync を配る
    // (取りこぼした変更を、実際に届いた変更より前に拾わせるため)。
    if (sawOpenSinceLastMessage) {
      sawOpenSinceLastMessage = false;
      dispatch({ type: "resync" });
    }
    try {
      const payload = JSON.parse(ev.data) as ViewerSseEvent;
      dispatch(payload);
    } catch {
      // 壊れたペイロードは無視する(次のイベントに任せる)
    }
  };
  es.onerror = () => {
    // EventSource は自動再接続するので何もしない。再接続後は onopen → 次の
    // onmessage の手前で resync が飛ぶ。
  };
  source = es;
  return es;
}

function dispatch(event: ViewerSseEvent) {
  for (const h of handlers) h(event);
}

/** SSE 購読を開始する。返り値を呼ぶと購読解除する(最後の1件が抜けても接続自体は閉じない)。 */
export function subscribeViewerEvents(handler: Handler): () => void {
  ensureSource();
  handlers.add(handler);
  return () => {
    handlers.delete(handler);
  };
}
