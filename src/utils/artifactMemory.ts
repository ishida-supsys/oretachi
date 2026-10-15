/**
 * React アーティファクト ↔ 親ウィンドウの postMessage ブリッジと、その上に載る
 * メモリー機能（フォーム入力などを復元するための JSON ストア）。
 *
 * ## なぜ postMessage しか経路が無いのか
 *
 * React アーティファクトの iframe は `sandbox="allow-scripts"`（`allow-same-origin` なし）
 * ＋ CSP `default-src 'none'` で動いているため、fetch も localStorage も Cookie も使えない
 * （設計の詳細は `reactArtifactSrcdoc.ts` のコメント参照）。したがって外へ出る手段は
 * `window.parent.postMessage` だけになる。
 *
 * CSP を緩めて MCP の HTTP エンドポイントを直接叩く案は採らない。AI が生成したコードへ
 * API キーを渡すことになるため。
 *
 * ## 送信元の判定
 *
 * opaque origin なので `event.origin` は `"null"` になり検証に使えない。親側は必ず
 * `event.source === iframe.contentWindow` で判定する（`artifactFrameLink.ts` と同じ）。
 *
 * ## プロトコル
 *
 * iframe → 親: `{ [REQUEST_MARKER]: true, requestId, method, params }`
 * 親 → iframe: `{ [RESULT_MARKER]: true, requestId, ok, result?, error? }`
 *
 * requestId + Promise + タイムアウトで包んでいるので、後続の MCP ツール呼び出しも
 * `method` を足すだけで同じブリッジに乗せられる。
 */

/** iframe → 親のリクエストを他のメッセージと区別するためのマーカー */
export const ARTIFACT_BRIDGE_REQUEST_MARKER = "__oretachiArtifactBridge";
/** 親 → iframe の応答マーカー */
export const ARTIFACT_BRIDGE_RESULT_MARKER = "__oretachiArtifactBridgeResult";
/**
 * 親 → iframe の一方向通知マーカー（リクエストへの応答ではないもの）。
 *
 * MCP の `artifact_store` が外からストアを書き換えたときに使う。押し込まないと、
 * iframe は起動時のスナップショットを持ち続け、次の 1 入力で自分の状態を丸ごと
 * 書き戻して外からの書き込みを消してしまう（MCP 側は成功を返しているので気づけない）。
 */
export const ARTIFACT_BRIDGE_PUSH_MARKER = "__oretachiArtifactBridgePush";
/** `ARTIFACT_BRIDGE_PUSH_MARKER` の通知種別: ストアが外から差し替わった */
export const ARTIFACT_BRIDGE_PUSH_MEMORY_CHANGED = "memory.changed";
/**
 * `ARTIFACT_BRIDGE_PUSH_MARKER` の通知種別: debounce 待ちの保存を今すぐ送り切れ（アプリのリロード前）。
 * iframe は保存が全部終わったら `ARTIFACT_BRIDGE_METHOD_FLUSHED` を送り返す。
 */
export const ARTIFACT_BRIDGE_PUSH_MEMORY_FLUSH = "memory.flush";

/** iframe → 親: `ARTIFACT_BRIDGE_PUSH_MEMORY_FLUSH` への完了通知。応答は返さない一方向のリクエスト */
export const ARTIFACT_BRIDGE_METHOD_FLUSHED = "memory.flushed";

/**
 * srcdoc ごとの nonce を iframe へ渡すグローバル名（任意）。
 *
 * iframe を差し替えても `frame.contentWindow` は同一オブジェクトのままなので、前の文書が仕掛けた
 * 遅延メッセージが新しい文書の分として届きうる（`artifactCspViolation.ts` と同じ問題）。
 * nonce があるとき、iframe は全リクエストに載せ、親は一致しないものを捨てる。親からの応答・通知にも
 * 載せ、iframe は一致しないものを捨てる（requestId が連番なので、前の文書宛の応答が新しい文書の
 * 同じ番号の待ちを解いてしまうのを防ぐ）。ブリッジは読み込み時に 1 回だけ読んでグローバルを消す。
 */
export const ARTIFACT_BRIDGE_NONCE_GLOBAL = "__oretachiBridgeNonce";

/** メモリー全体を保存する。params は `{ memory: object }` */
export const ARTIFACT_BRIDGE_METHOD_MEMORY_SET = "memory.set";

/**
 * oretachi の MCP ツールを呼ぶ。params は `{ tool: string, params: object }`。
 *
 * ホワイトリストとスコープの強制は Rust 側（`mcp_server::call_tool_for_artifact`）で行う。
 * ここで弾かないのは、許可ツールの一覧を srcdoc へ埋め込むと
 * 「フロントの一覧が真の権限」だと誤解される作りになるため（唯一の関門は Rust）。
 */
export const ARTIFACT_BRIDGE_METHOD_MCP_CALL = "mcp.call";

/** `ARTIFACT_BRIDGE_PUSH_MARKER` の通知種別: 専用 AI ターミナルのフックイベント（Stop など）。`payload` に中身 */
export const ARTIFACT_BRIDGE_PUSH_AI_TERMINAL_EVENT = "aiTerminal.event";

/**
 * 専用 AI ターミナル API の method 接頭辞。`aiTerminal.open` / `.write` / `.sendKeys` / `.read` /
 * `.inspectPrompt` / `.answerPrompt` / `.close` / `.list`。params は Rust コマンドの引数
 * （`terminalId` など）をそのまま持つ。権限の関門は Rust 側（`ai_terminal`）で、ここは中継だけ。
 */
export const ARTIFACT_BRIDGE_METHOD_AI_TERMINAL_PREFIX = "aiTerminal.";

/** `ARTIFACT_BRIDGE_PUSH_MARKER` の通知種別: 自分が開いたサブウィンドウがユーザーに閉じられた。`payload` は `{ event: "closed", id }` */
export const ARTIFACT_BRIDGE_PUSH_SUB_WINDOW_EVENT = "subWindow.event";

/**
 * 専用サブウィンドウ API の method 接頭辞。`subWindow.open` / `.close` / `.list` / `.setPosition` /
 * `.setSize` / `.startDrag`。params は Rust コマンドの引数に写す（`subWindowBridge.ts`）。
 * 権限（manifest の `subWindow`）・上限・見た目の検査は Rust 側（`app_subwindow`）の関門で、ここは中継だけ。
 */
export const ARTIFACT_BRIDGE_METHOD_SUB_WINDOW_PREFIX = "subWindow.";

/**
 * アプリ（`app` 付きアーティファクト）のファイルシステム API。パスは `worktree://<相対>` /
 * `artifact://<相対>` だけ。判定は Rust 側（`app_fs::resolve_scoped_path`）。通常のビューアはこれらを
 * 処理しないので「使えない API」として失敗する。
 *
 * - `fs.read`:   `{ path, encoding: "utf8" | "base64" }` → 本文の文字列
 * - `fs.write`:  `{ path, data, encoding, createDirs }` → 書き込んだバイト数
 * - `fs.list`:   `{ path }` → `{ entries: [{ name, kind, size, modifiedMs }], truncated }`
 * - `fs.remove`: `{ path }`（ファイルのみ）
 */
export const ARTIFACT_BRIDGE_METHOD_FS_READ = "fs.read";
export const ARTIFACT_BRIDGE_METHOD_FS_WRITE = "fs.write";
export const ARTIFACT_BRIDGE_METHOD_FS_LIST = "fs.list";
export const ARTIFACT_BRIDGE_METHOD_FS_REMOVE = "fs.remove";

/**
 * アプリの SQLite API。DB ファイルの場所は FS と同じ `worktree://` / `artifact://` のパスで、
 * 判定は Rust 側（`app_db`）。通常のビューアは処理しないので「使えない API」として失敗する。
 *
 * - `db.query`:   `{ path, sql, params? }` → `{ columns, rows }`（rows は行ごとの配列）
 * - `db.execute`: `{ path, sql, params? }` → `{ rowsAffected, lastInsertRowid }`
 * - `db.close`:   `{ path }` → 閉じたら true（接続は host が持つ。停止まで開いたままでもよい）
 *
 * params は null / 真偽値 / 数値 / 文字列 / `{ $blob: base64 }`。BLOB は結果でも `{ $blob: base64 }`。
 */
export const ARTIFACT_BRIDGE_METHOD_DB_QUERY = "db.query";
export const ARTIFACT_BRIDGE_METHOD_DB_EXECUTE = "db.execute";
export const ARTIFACT_BRIDGE_METHOD_DB_CLOSE = "db.close";

/**
 * アプリのネットワーク API。iframe の CSP は `connect-src 'none'` のまま、通信は Rust のプロキシ
 * （`app_net`）が出す。manifest の `network.allow`（URL プレフィックス）の外・リダイレクト先・
 * ループバック／プライベート IP・oretachi 自身のポートは Rust 側が拒否する。
 *
 * - `net.fetch`: `{ url, init? }` → `{ status, statusText, url, headers, body, bodyEncoding }`
 *   - init: `{ method?, headers?（[[名前, 値], …] かオブジェクト）, body?, bodyEncoding?: "utf8"|"base64", responseType?: "text"|"base64" }`
 *   - 4xx / 5xx でも reject せず status で返す（fetch と同じ）。拒否・上限超過・タイムアウトは reject
 */
export const ARTIFACT_BRIDGE_METHOD_NET_FETCH = "net.fetch";

/** 応答が返らないまま Promise が残り続けないようにするタイムアウト（iframe 側） */
const BRIDGE_TIMEOUT_MS = 10000;
/**
 * 時間のかかる呼び出し（端末の起動待ち 8 秒、ダイアログ回答後の再描画待ち 3 秒〜）のタイムアウト。
 * 既定の 10 秒だと、Rust 側がまだ処理中なのに iframe だけが先に失敗してしまう。
 */
const BRIDGE_LONG_TIMEOUT_MS = 30000;
/**
 * 1 文字入力ごとに保存が飛ばないようにする debounce（iframe 側）。
 *
 * この間に iframe が消えると直前の入力は落ちる。`pagehide` で送り切ることはできない
 * （親は iframe の消滅で `frame.contentWindow` との同一性判定に失敗し、
 * 消えかけのフレームからのメッセージを受け取れない）。そのため iframe を壊さない側で
 * 手当てしている: Preview / Code の切替では iframe を v-show で残し、
 * アーティファクトの切り替えでのみ作り直す（落ちるのは最後の 400ms 以内の入力だけ）。
 */
const MEMORY_FLUSH_DEBOUNCE_MS = 400;
/**
 * メモリーのサイズ上限。判定の本体は Rust 側（`ARTIFACT_MEMORY_MAX_BYTES`）で、
 * ここは無駄な IPC 往復を省くための早期判定。JS は UTF-16 コードユニット数を数えるので
 * マルチバイト文字ではここを通っても Rust 側で落ちる（Rust が最終判断）。
 */
export const ARTIFACT_MEMORY_MAX_BYTES = 1024 * 1024;

export interface ArtifactBridgeRequest {
  requestId: string;
  method: string;
  params: Record<string, unknown>;
}

/**
 * `message` イベントが対象 iframe から来たブリッジのリクエストなら中身を返す。
 * そうでなければ null。
 */
export function readArtifactBridgeRequest(
  event: MessageEvent,
  frame: HTMLIFrameElement | null,
  /** 指定すると、この nonce を載せていないリクエスト（前の文書からの遅延メッセージ）を捨てる */
  expectedNonce?: string,
): ArtifactBridgeRequest | null {
  if (!frame || event.source !== frame.contentWindow) return null;
  const data = event.data as Record<string, unknown> | null;
  if (!data || typeof data !== "object") return null;
  if (data[ARTIFACT_BRIDGE_REQUEST_MARKER] !== true) return null;
  if (expectedNonce !== undefined && data.nonce !== expectedNonce) return null;
  const requestId = data.requestId;
  const method = data.method;
  if (typeof requestId !== "string" || typeof method !== "string") return null;
  const params = data.params;
  return {
    requestId,
    method,
    params: params && typeof params === "object" ? (params as Record<string, unknown>) : {},
  };
}

/** リクエストへの応答を iframe へ返す。iframe は opaque origin なので targetOrigin は "*" */
export function postArtifactBridgeResult(
  frame: HTMLIFrameElement | null,
  requestId: string,
  outcome: { ok: true; result?: unknown } | { ok: false; error: string },
  /** 指定すると応答に載せる。iframe は自分の nonce と違う応答を捨てる */
  nonce?: string,
): void {
  const target = frame?.contentWindow;
  if (!target) return;
  try {
    target.postMessage(
      {
        [ARTIFACT_BRIDGE_RESULT_MARKER]: true,
        requestId,
        ...(nonce !== undefined ? { nonce } : {}),
        ...outcome,
      },
      "*",
    );
  } catch {
    // iframe が既に差し替わっている場合など。応答先が居ないだけなので無視する
  }
}

/**
 * 外からストアが書き換わったことを iframe へ知らせる。
 * iframe 側は `state` を差し替えて `subscribeMemory` / `useMemory` を再通知する。
 *
 * **JSON を1往復させてから送る。** 呼び出し側が渡してくるのは Vue の `states` ref
 * 由来の値で、reactive Proxy を `postMessage` に渡すと構造化複製が
 * `DataCloneError` で落ちる（Proxy は複製できない）。ストアの中身は Rust 側の JSON
 * サイドカーと往復する契約なので、ここで素の JSON へ落として構わない。
 */
export function postArtifactBridgeMemoryChanged(
  frame: HTMLIFrameElement | null,
  memory: Record<string, unknown>,
  nonce?: string,
): void {
  const target = frame?.contentWindow;
  if (!target) return;
  let plain: Record<string, unknown>;
  try {
    plain = JSON.parse(JSON.stringify(memory ?? {}));
  } catch (e) {
    // 循環参照など。押し込めないと iframe が古い値を書き戻して外の更新を消すので、
    // 黙って捨てずに残す
    console.warn("artifact store の push をシリアライズできませんでした", e);
    return;
  }
  try {
    target.postMessage(
      {
        [ARTIFACT_BRIDGE_PUSH_MARKER]: true,
        event: ARTIFACT_BRIDGE_PUSH_MEMORY_CHANGED,
        memory: plain,
        ...(nonce !== undefined ? { nonce } : {}),
      },
      "*",
    );
  } catch (e) {
    // iframe が既に差し替わっている場合など。押し込みは「届かないと外の更新が消える」
    // 経路なので、応答（postArtifactBridgeResult）と違って痕跡を残す
    console.warn("artifact store の push を送れませんでした", e);
  }
}

/** 専用 AI ターミナルのフックイベント（Stop など）を iframe へ知らせる。応答の無い一方向通知 */
export function postArtifactBridgeAiTerminalEvent(
  frame: HTMLIFrameElement | null,
  payload: Record<string, unknown>,
  nonce?: string,
): void {
  const target = frame?.contentWindow;
  if (!target) return;
  try {
    target.postMessage(
      {
        [ARTIFACT_BRIDGE_PUSH_MARKER]: true,
        event: ARTIFACT_BRIDGE_PUSH_AI_TERMINAL_EVENT,
        payload: JSON.parse(JSON.stringify(payload ?? {})),
        ...(nonce !== undefined ? { nonce } : {}),
      },
      "*",
    );
  } catch (e) {
    console.warn("AI ターミナルのイベントを iframe へ送れませんでした", e);
  }
}

/** サブウィンドウのイベント（閉じられた）を iframe へ知らせる。応答の無い一方向通知 */
export function postArtifactBridgeSubWindowEvent(
  frame: HTMLIFrameElement | null,
  payload: Record<string, unknown>,
  nonce?: string,
): void {
  const target = frame?.contentWindow;
  if (!target) return;
  try {
    target.postMessage(
      {
        [ARTIFACT_BRIDGE_PUSH_MARKER]: true,
        event: ARTIFACT_BRIDGE_PUSH_SUB_WINDOW_EVENT,
        payload: JSON.parse(JSON.stringify(payload ?? {})),
        ...(nonce !== undefined ? { nonce } : {}),
      },
      "*",
    );
  } catch (e) {
    console.warn("サブウィンドウのイベントを iframe へ送れませんでした", e);
  }
}

/** debounce 待ちの保存を送り切るよう iframe へ頼む（アプリのリロード前）。完了は `ARTIFACT_BRIDGE_METHOD_FLUSHED` */
export function postArtifactBridgeFlush(frame: HTMLIFrameElement | null, nonce?: string): void {
  const target = frame?.contentWindow;
  if (!target) return;
  try {
    target.postMessage(
      {
        [ARTIFACT_BRIDGE_PUSH_MARKER]: true,
        event: ARTIFACT_BRIDGE_PUSH_MEMORY_FLUSH,
        ...(nonce !== undefined ? { nonce } : {}),
      },
      "*",
    );
  } catch (e) {
    console.warn("メモリーの flush 要求を送れませんでした", e);
  }
}

/**
 * エクスポートした単体 HTML（zip の `view.html`）であることを示すグローバルフラグ。
 * `reactArtifactSrcdoc.ts` がブリッジより前にこれを立てる。
 *
 * 単体ファイルには応答を返す親がいないため、そのままだとメモリー保存が
 * タイムアウトするまで（10 秒）ぶら下がり、`callTool` は必ず失敗する。
 * フラグが立っているときはメモリーをメモリ上だけで完結させ（保存は即 resolve）、
 * ブリッジ越しの呼び出しは「エクスポートされたファイルでは使えない」と即座に断る。
 */
export const ARTIFACT_STANDALONE_FLAG = "__oretachiStandalone";

/**
 * iframe 内に注入するブリッジ本体。`window.__oretachi` を定義し、
 * `reactArtifactSrcdoc.ts` の makeRequire が `require('oretachi')` として返す。
 *
 * 初期値は同期で読めなければ初回レンダリングでフォームを復元できないため、
 * `_source` / `_modules` と同じく `_memory` textarea から同期で読む。
 * 書き込みだけが postMessage + debounce になる。
 */
export const ARTIFACT_BRIDGE_JS =
  "(function(){" +
  "  var standalone=window[" + JSON.stringify(ARTIFACT_STANDALONE_FLAG) + "]===true;" +
  // srcdoc ごとの nonce（任意）。読んだらグローバルは消す
  "  var NONCE=window[" + JSON.stringify(ARTIFACT_BRIDGE_NONCE_GLOBAL) + "];" +
  "  try{delete window[" + JSON.stringify(ARTIFACT_BRIDGE_NONCE_GLOBAL) + "];}catch(err){}" +
  "  var hasNonce=typeof NONCE==='string';" +
  // ── リクエスト/レスポンスの土台（メモリー以外の method も後からここに乗る）──
  "  var pending={};" +
  "  var seq=0;" +
  "  window.addEventListener('message',function(e){" +
  // 親が送信元を検証しているのと対称に、応答は親からのものだけ受理する
  // （requestId は連番なので、同一ウィンドウ内の別 iframe に成功を偽装され得る）
  "    if(e.source!==parent)return;" +
  "    var d=e.data;" +
  "    if(!d||typeof d!=='object')return;" +
  // 前の文書宛の応答・通知は捨てる（nonce が無い文書では従来どおり何も見ない）
  "    if(hasNonce&&d.nonce!==NONCE)return;" +
  // 一方向通知（ストアの外部更新・リロード前の flush 要求）。pending は触らない
  "    if(d[" + JSON.stringify(ARTIFACT_BRIDGE_PUSH_MARKER) + "]===true){" +
  "      if(d.event===" + JSON.stringify(ARTIFACT_BRIDGE_PUSH_MEMORY_CHANGED) + ")" +
  "        applyExternalMemory(d.memory);" +
  "      else if(d.event===" + JSON.stringify(ARTIFACT_BRIDGE_PUSH_MEMORY_FLUSH) + ")" +
  "        flushNow();" +
  "      else if(d.event===" + JSON.stringify(ARTIFACT_BRIDGE_PUSH_AI_TERMINAL_EVENT) + ")" +
  "        emitAiTerminalEvent(d.payload);" +
  "      else if(d.event===" + JSON.stringify(ARTIFACT_BRIDGE_PUSH_SUB_WINDOW_EVENT) + ")" +
  "        emitSubWindowEvent(d.payload);" +
  "      return;" +
  "    }" +
  "    if(d[" + JSON.stringify(ARTIFACT_BRIDGE_RESULT_MARKER) + "]!==true)return;" +
  "    var p=pending[d.requestId];" +
  "    if(!p)return;" +
  "    delete pending[d.requestId];" +
  "    clearTimeout(p.timer);" +
  "    if(d.ok)p.resolve(d.result);" +
  "    else p.reject(new Error(String(d.error||'oretachi bridge error')));" +
  "  });" +
  "  function call(method,params,timeoutMs){" +
  "    if(standalone)return Promise.reject(new Error(" +
  "      'oretachi bridge is unavailable in an exported file: '+method));" +
  "    return new Promise(function(resolve,reject){" +
  "      var id='r'+(++seq);" +
  "      var timer=setTimeout(function(){" +
  "        delete pending[id];" +
  "        reject(new Error('oretachi bridge timeout: '+method));" +
  "      },(typeof timeoutMs==='number'?timeoutMs:" + BRIDGE_TIMEOUT_MS + "));" +
  "      pending[id]={resolve:resolve,reject:reject,timer:timer};" +
  "      try{" +
  "        var msg={requestId:id,method:method,params:params||{}};" +
  "        msg[" + JSON.stringify(ARTIFACT_BRIDGE_REQUEST_MARKER) + "]=true;" +
  "        if(hasNonce)msg.nonce=NONCE;" +
  "        parent.postMessage(msg,'*');" +
  "      }catch(err){" +
  "        delete pending[id];" +
  "        clearTimeout(timer);" +
  "        reject(err);" +
  "      }" +
  "    });" +
  "  }" +
  // ── メモリー ──
  "  var state={};" +
  "  try{" +
  "    var el=document.getElementById('_memory');" +
  "    var parsed=JSON.parse((el&&el.value)||'{}');" +
  "    if(parsed&&typeof parsed==='object'&&!Array.isArray(parsed))state=parsed;" +
  "  }catch(err){}" +
  "  var listeners=[];" +
  "  function subscribe(fn){" +
  "    listeners.push(fn);" +
  "    return function(){listeners=listeners.filter(function(f){return f!==fn;});};" +
  "  }" +
  "  function notify(){" +
  "    listeners.slice().forEach(function(fn){try{fn(state);}catch(err){}});" +
  "  }" +
  // debounce 中の setMemory はまとめて 1 回の保存にし、その 1 回の結果を全員へ返す。
  // さらに保存は必ず 1 本ずつにする（IPC の往復が debounce より長引いたときに
  // 2 本並走させると、古いスナップショットが後着して lost update になる）
  "  var flushTimer=null;" +
  "  var waiters=[];" +
  "  var inflight=false;" +
  // リロード前の flush 要求。保存が全部終わった（debounce 待ちも送信中も無い）ら通知する
  "  var idleWaiters=[];" +
  "  function checkIdle(){" +
  "    if(inflight||waiters.length>0||flushTimer!==null)return;" +
  "    var l=idleWaiters;idleWaiters=[];" +
  "    l.forEach(function(f){f();});" +
  "  }" +
  "  function flush(){" +
  "    flushTimer=null;" +
  "    if(inflight||waiters.length===0)return;" +
  "    var batch=waiters;" +
  "    waiters=[];" +
  // 上限超過は往復させずここで落とす（Rust 側でも同じ判定をしている）
  "    var json=JSON.stringify(state);" +
  "    if(json.length>" + ARTIFACT_MEMORY_MAX_BYTES + "){" +
  "      var tooLarge=new Error('oretachi memory too large: '+json.length+' > '+" +
  ARTIFACT_MEMORY_MAX_BYTES + ");" +
  "      batch.forEach(function(w){w.reject(tooLarge);});" +
  "      return;" +
  "    }" +
  // 単体ファイルには保存先が無い。メモリ上の state はそのまま生きているので、
  // 「保存できた」ことにして UI を止めない（次に開いたときに残らないだけ）
  "    if(standalone){batch.forEach(function(w){w.resolve();});return;}" +
  "    inflight=true;" +
  // 前の保存が返ってから、その間に積まれた分をまとめて送り直す
  "    var done=function(){inflight=false;if(waiters.length>0)flush();checkIdle();};" +
  "    call(" + JSON.stringify(ARTIFACT_BRIDGE_METHOD_MEMORY_SET) + ",{memory:state}).then(function(){" +
  "      batch.forEach(function(w){w.resolve();});" +
  "      done();" +
  "    },function(err){" +
  "      batch.forEach(function(w){w.reject(err);});" +
  "      done();" +
  "    });" +
  "  }" +
  "  function flushNow(){" +
  "    if(standalone)return;" +
  "    if(flushTimer!==null){clearTimeout(flushTimer);flushTimer=null;}" +
  "    idleWaiters.push(function(){" +
  "      try{" +
  "        var m={requestId:'f'+(++seq),method:" + JSON.stringify(ARTIFACT_BRIDGE_METHOD_FLUSHED) + ",params:{}};" +
  "        m[" + JSON.stringify(ARTIFACT_BRIDGE_REQUEST_MARKER) + "]=true;" +
  "        if(hasNonce)m.nonce=NONCE;" +
  "        parent.postMessage(m,'*');" +
  "      }catch(err){}" +
  "    });" +
  "    flush();" +
  "    checkIdle();" +
  "  }" +
  "  function schedule(){" +
  "    return new Promise(function(resolve,reject){" +
  "      waiters.push({resolve:resolve,reject:reject});" +
  "      if(flushTimer!==null)clearTimeout(flushTimer);" +
  "      flushTimer=setTimeout(flush," + MEMORY_FLUSH_DEBOUNCE_MS + ");" +
  "    });" +
  "  }" +
  // 外から差し替わったストアを取り込む。全置換（ストアの意味論が全置換なので）。
  // ユーザーが入力中のキーも上書きされうるが、それを避けたいアーティファクトは
  // `locked_while_open` を宣言して MCP からの書き込みそのものを止める
  "  function applyExternalMemory(next){" +
  "    if(!next||typeof next!=='object'||Array.isArray(next))return;" +
  "    state=next;" +
  // debounce 待ちだった入力は押し流された。そのまま flush すると押し込まれた値を
  // 送って「保存できた」と resolve してしまう（呼び出し側の値はどこにも残らない）。
  // 保存されなかったことを reject で伝える
  "    if(flushTimer!==null){clearTimeout(flushTimer);flushTimer=null;}" +
  "    var dropped=waiters;" +
  "    waiters=[];" +
  "    if(dropped.length>0){" +
  "      var err=new Error('oretachi memory was replaced from outside before the pending save');" +
  "      dropped.forEach(function(w){w.reject(err);});" +
  "    }" +
  "    notify();" +
  "  }" +
  "  function getMemory(){return state;}" +
  "  function replace(next){" +
  "    if(!next||typeof next!=='object'||Array.isArray(next))" +
  "      throw new Error('setMemory expects a plain object');" +
  "    state=next;" +
  "    notify();" +
  "    return schedule();" +
  "  }" +
  "  function setMemory(next){" +
  "    return replace((typeof next==='function')?next(state):next);" +
  "  }" +
  "  function patch(key,value){" +
  "    var next={};" +
  "    for(var k in state)if(Object.prototype.hasOwnProperty.call(state,k))next[k]=state[k];" +
  "    if(value===undefined)delete next[key];" +
  "    else next[key]=value;" +
  "    return replace(next);" +
  "  }" +
  "  function clearMemory(){return replace({});}" +
  // useState + subscribe で組む。更新関数は state を先に書き換えてから notify し、
  // その通知でローカル state を追従させる（レンダー中に副作用を起こさないため）
  "  function useMemory(key,initialValue){" +
  "    var R=window.React;" +
  "    if(!R)throw new Error('useMemory requires React');" +
  "    var read=function(){return state[key]!==undefined?state[key]:initialValue;};" +
  "    var pair=R.useState(read);" +
  "    var value=pair[0],setValue=pair[1];" +
  "    var initialRef=R.useRef(initialValue);" +
  // レンダー中に書き換えないこと（副作用になる）。初期値が変わるのは稀なので effect で追う
  "    R.useEffect(function(){initialRef.current=initialValue;});" +
  "    R.useEffect(function(){" +
  "      setValue(read());" +
  "      return subscribe(function(next){" +
  // キーが消えた（clearMemory / setMemory での差し替え）ときは初期値へ戻す
  "        var has=Object.prototype.hasOwnProperty.call(next,key)&&next[key]!==undefined;" +
  "        var v=has?next[key]:initialRef.current;" +
  "        setValue(function(prev){return prev===v?prev:v;});" +
  "      });" +
  "    },[key]);" +
  "    var update=R.useCallback(function(next){" +
  "      var cur=state[key]!==undefined?state[key]:initialRef.current;" +
  "      var v=(typeof next==='function')?next(cur):next;" +
  // 保存の失敗（上限超過など）は握り潰さず、呼び出し側が拾えるよう Promise を返す
  "      return patch(key,v);" +
  "    },[key]);" +
  "    return [value,update];" +
  "  }" +
  // ── MCP ツール呼び出し ──
  // 応答は Rust 側ツールの戻り値テキスト。JSON を返すツールが多いので、
  // パースできたらオブジェクトで返し、できなければ文字列のまま返す
  "  function callTool(tool,params){" +
  "    if(typeof tool!=='string'||tool==='')" +
  "      return Promise.reject(new Error('callTool expects a tool name'));" +
  "    return call(" + JSON.stringify(ARTIFACT_BRIDGE_METHOD_MCP_CALL) + "," +
  "      {tool:tool,params:params||{}}).then(function(text){" +
  "      if(typeof text!=='string')return text;" +
  "      try{return JSON.parse(text);}catch(err){return text;}" +
  "    });" +
  "  }" +
  // ── ファイルシステム（アプリのみ。承認された範囲の外は Rust 側が拒否する）──
  "  function bytesToB64(u8){" +
  "    var s='',CH=0x8000;" +
  "    for(var i=0;i<u8.length;i+=CH)s+=String.fromCharCode.apply(null,u8.subarray(i,i+CH));" +
  "    return btoa(s);" +
  "  }" +
  "  function b64ToBytes(b){" +
  "    var s=atob(b),u=new Uint8Array(s.length);" +
  "    for(var i=0;i<s.length;i++)u[i]=s.charCodeAt(i);" +
  "    return u;" +
  "  }" +
  "  function fsPath(path){" +
  "    if(typeof path!=='string'||path==='')" +
  "      return Promise.reject(new Error('fs expects a path like worktree://dir/file or artifact://file'));" +
  "    return null;" +
  "  }" +
  "  function fsWrite(path,data,encoding,opts){" +
  "    return fsPath(path)||call(" + JSON.stringify(ARTIFACT_BRIDGE_METHOD_FS_WRITE) + "," +
  "      {path:path,data:data,encoding:encoding,createDirs:!!(opts&&opts.createDirs)});" +
  "  }" +
  "  var fs={" +
  "    readText:function(path){" +
  "      return fsPath(path)||call(" + JSON.stringify(ARTIFACT_BRIDGE_METHOD_FS_READ) + "," +
  "        {path:path,encoding:'utf8'});" +
  "    }," +
  "    readBytes:function(path){" +
  "      return (fsPath(path)||call(" + JSON.stringify(ARTIFACT_BRIDGE_METHOD_FS_READ) + "," +
  "        {path:path,encoding:'base64'})).then(b64ToBytes);" +
  "    }," +
  "    writeText:function(path,text,opts){return fsWrite(path,String(text),'utf8',opts);}," +
  "    writeBytes:function(path,bytes,opts){" +
  "      var u8=bytes instanceof Uint8Array?bytes:new Uint8Array(bytes);" +
  "      return fsWrite(path,bytesToB64(u8),'base64',opts);" +
  "    }," +
  "    list:function(path){" +
  "      return fsPath(path)||call(" + JSON.stringify(ARTIFACT_BRIDGE_METHOD_FS_LIST) + ",{path:path});" +
  "    }," +
  "    remove:function(path){" +
  "      return fsPath(path)||call(" + JSON.stringify(ARTIFACT_BRIDGE_METHOD_FS_REMOVE) + ",{path:path});" +
  "    }" +
  "  };" +
  // ── データベース（アプリのみ。承認された範囲の外・禁止された SQL は Rust 側が拒否する）──
  "  function dbCall(method,path,sql,params){" +
  "    if(typeof path!=='string'||path==='')" +
  "      return Promise.reject(new Error('db expects a path like worktree://dir/app.db or artifact://app.db'));" +
  "    if(method!==" + JSON.stringify(ARTIFACT_BRIDGE_METHOD_DB_CLOSE) + "&&typeof sql!=='string')" +
  "      return Promise.reject(new Error('db expects an SQL string'));" +
  "    if(params!=null&&!Array.isArray(params))" +
  "      return Promise.reject(new Error('db params must be an array'));" +
  "    return call(method,{path:path,sql:sql,params:params||[]});" +
  "  }" +
  "  var db={" +
  "    query:function(path,sql,params){return dbCall(" + JSON.stringify(ARTIFACT_BRIDGE_METHOD_DB_QUERY) + ",path,sql,params);}," +
  "    execute:function(path,sql,params){return dbCall(" + JSON.stringify(ARTIFACT_BRIDGE_METHOD_DB_EXECUTE) + ",path,sql,params);}," +
  "    close:function(path){return dbCall(" + JSON.stringify(ARTIFACT_BRIDGE_METHOD_DB_CLOSE) + ",path);}" +
  "  };" +
  // ── ネットワーク（manifest の `network` 権限が承認されたアプリだけが使える）──
  // 通信は Rust のプロキシが出す。許可・拒否の判断は Rust 側で、ここは引数の形を整えるだけ
  "  function netHeaders(h){" +
  "    if(h==null)return [];" +
  "    if(Array.isArray(h)){" +
  "      for(var i=0;i<h.length;i++){if(!Array.isArray(h[i])||h[i].length!==2)return null;}" +
  "      return h.map(function(p){return [String(p[0]),String(p[1])];});" +
  "    }" +
  "    if(typeof h==='object')return Object.keys(h).map(function(k){return [k,String(h[k])];});" +
  "    return null;" +
  "  }" +
  "  var net={" +
  "    fetch:function(url,init){" +
  "      if(typeof url!=='string'||url==='')" +
  "        return Promise.reject(new Error('net.fetch expects an absolute URL string'));" +
  "      init=init||{};" +
  "      var headers=netHeaders(init.headers);" +
  "      if(headers===null)return Promise.reject(new Error('net.fetch headers must be an object or an array of [name, value]'));" +
  "      var p={url:url,method:init.method,headers:headers,body:init.body,bodyEncoding:init.bodyEncoding,responseType:init.responseType};" +
  "      if(p.body!=null&&typeof p.body!=='string')" +
  "        return Promise.reject(new Error('net.fetch body must be a string (for binary, pass base64 text with bodyEncoding base64)'));" +
  "      return call(" + JSON.stringify(ARTIFACT_BRIDGE_METHOD_NET_FETCH) + ",p," + BRIDGE_LONG_TIMEOUT_MS + ");" +
  "    }" +
  "  };" +
  // ── 専用 AI ターミナル（manifest の `aiTerminal` 権限が承認されたアプリだけが使える）──
  // 権限の関門は Rust 側。ここは中継するだけ。戻り値は JSON ならオブジェクトで返る
  "  var aiListeners=[];" +
  "  function emitAiTerminalEvent(p){" +
  "    aiListeners.slice().forEach(function(fn){try{fn(p);}catch(err){}});" +
  "  }" +
  "  function ai(name,params,long){" +
  "    return call(" + JSON.stringify(ARTIFACT_BRIDGE_METHOD_AI_TERMINAL_PREFIX) + "+name,params||{}," +
  "      long?" + BRIDGE_LONG_TIMEOUT_MS + ":undefined);" +
  "  }" +
  "  var aiTerminal={" +
  "    open:function(opts){return ai('open',opts,true);}," +
  "    write:function(terminalId,text,submit){" +
  "      var p={terminalId:terminalId,text:text};" +
  "      if(submit!==undefined)p.submit=submit;" +
  "      return ai('write',p);" +
  "    }," +
  "    sendKeys:function(terminalId,keys){return ai('sendKeys',{terminalId:terminalId,keys:keys});}," +
  "    read:function(terminalId,opts){" +
  "      var p={terminalId:terminalId};" +
  "      if(opts&&opts.maxBytes!==undefined)p.maxBytes=opts.maxBytes;" +
  "      if(opts&&opts.fromCursor!==undefined)p.fromCursor=opts.fromCursor;" +
  "      return ai('read',p);" +
  "    }," +
  "    inspectPrompt:function(terminalId){return ai('inspectPrompt',{terminalId:terminalId});}," +
  "    answerPrompt:function(terminalId,answer){return ai('answerPrompt',{terminalId:terminalId,answer:answer},true);}," +
  "    close:function(terminalId){return ai('close',{terminalId:terminalId});}," +
  "    list:function(){return ai('list',{});}," +
  "    onEvent:function(fn){" +
  "      aiListeners.push(fn);" +
  "      return function(){aiListeners=aiListeners.filter(function(f){return f!==fn;});};" +
  "    }" +
  "  };" +
  // ── 専用サブウィンドウ（manifest の `subWindow` 権限が承認されたアプリだけが使える）──
  // 中身は同じアプリ（このコードが、開いたウィンドウの中でも動く）。`info` は開かれた側だけ非 null
  // （`{ id, params }`）なので、それで描き分ける。権限・上限・見た目の関門は Rust 側。ここは中継だけ
  "  var subInfo=null;" +
  "  try{" +
  "    var subEl=document.getElementById('_subwindow');" +
  "    if(subEl&&subEl.value)subInfo=JSON.parse(subEl.value);" +
  "  }catch(err){subInfo=null;}" +
  "  var swListeners=[];" +
  "  function emitSubWindowEvent(p){" +
  "    swListeners.slice().forEach(function(fn){try{fn(p);}catch(err){}});" +
  "  }" +
  "  function sw(name,params){" +
  "    return call(" + JSON.stringify(ARTIFACT_BRIDGE_METHOD_SUB_WINDOW_PREFIX) + "+name,params||{});" +
  "  }" +
  "  var subWindow={" +
  "    info:subInfo," +
  "    open:function(opts){return sw('open',opts||{});}," +
  "    close:function(id){return sw('close',{id:id});}," +
  "    list:function(){return sw('list',{});}," +
  "    setPosition:function(id,x,y){return sw('setPosition',{id:id,x:x,y:y});}," +
  "    setSize:function(id,width,height){return sw('setSize',{id:id,width:width,height:height});}," +
  // マウスが押されている間に呼ぶこと（mousedown ハンドラから）。枠なしウィンドウをドラッグで動かす
  "    startDrag:function(){return sw('startDrag',{});}," +
  "    onClosed:function(fn){" +
  "      swListeners.push(fn);" +
  "      return function(){swListeners=swListeners.filter(function(f){return f!==fn;});};" +
  "    }" +
  "  };" +
  "  window.__oretachi={" +
  "    getMemory:getMemory," +
  "    setMemory:setMemory," +
  "    setMemoryKey:patch," +
  "    clearMemory:clearMemory," +
  "    useMemory:useMemory," +
  "    subscribeMemory:subscribe," +
  "    callTool:callTool," +
  "    fs:fs," +
  "    db:db," +
  "    net:net," +
  "    call:call," +
  "    aiTerminal:aiTerminal," +
  "    subWindow:subWindow" +
  "  };" +
  // `state` は setMemory ごとに別オブジェクトへ差し替わるので、スナップショットを
  // 掴ませないようゲッターにする（`oretachi.memory` は常に最新を指す）
  "  Object.defineProperty(window.__oretachi,'memory',{get:getMemory,enumerable:true});" +
  "})();";
