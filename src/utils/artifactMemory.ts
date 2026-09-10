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

/** 応答が返らないまま Promise が残り続けないようにするタイムアウト（iframe 側） */
const BRIDGE_TIMEOUT_MS = 10000;
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
): ArtifactBridgeRequest | null {
  if (!frame || event.source !== frame.contentWindow) return null;
  const data = event.data as Record<string, unknown> | null;
  if (!data || typeof data !== "object") return null;
  if (data[ARTIFACT_BRIDGE_REQUEST_MARKER] !== true) return null;
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
): void {
  const target = frame?.contentWindow;
  if (!target) return;
  try {
    target.postMessage(
      {
        [ARTIFACT_BRIDGE_RESULT_MARKER]: true,
        requestId,
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
      },
      "*",
    );
  } catch (e) {
    // iframe が既に差し替わっている場合など。押し込みは「届かないと外の更新が消える」
    // 経路なので、応答（postArtifactBridgeResult）と違って痕跡を残す
    console.warn("artifact store の push を送れませんでした", e);
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
  // ── リクエスト/レスポンスの土台（メモリー以外の method も後からここに乗る）──
  "  var pending={};" +
  "  var seq=0;" +
  "  window.addEventListener('message',function(e){" +
  // 親が送信元を検証しているのと対称に、応答は親からのものだけ受理する
  // （requestId は連番なので、同一ウィンドウ内の別 iframe に成功を偽装され得る）
  "    if(e.source!==parent)return;" +
  "    var d=e.data;" +
  "    if(!d||typeof d!=='object')return;" +
  // 一方向通知（ストアの外部更新）。pending は触らない
  "    if(d[" + JSON.stringify(ARTIFACT_BRIDGE_PUSH_MARKER) + "]===true){" +
  "      if(d.event===" + JSON.stringify(ARTIFACT_BRIDGE_PUSH_MEMORY_CHANGED) + ")" +
  "        applyExternalMemory(d.memory);" +
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
  "  function call(method,params){" +
  "    if(standalone)return Promise.reject(new Error(" +
  "      'oretachi bridge is unavailable in an exported file: '+method));" +
  "    return new Promise(function(resolve,reject){" +
  "      var id='r'+(++seq);" +
  "      var timer=setTimeout(function(){" +
  "        delete pending[id];" +
  "        reject(new Error('oretachi bridge timeout: '+method));" +
  "      }," + BRIDGE_TIMEOUT_MS + ");" +
  "      pending[id]={resolve:resolve,reject:reject,timer:timer};" +
  "      try{" +
  "        parent.postMessage({" + JSON.stringify(ARTIFACT_BRIDGE_REQUEST_MARKER) + ":true," +
  "          requestId:id,method:method,params:params||{}},'*');" +
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
  "    var done=function(){inflight=false;if(waiters.length>0)flush();};" +
  "    call(" + JSON.stringify(ARTIFACT_BRIDGE_METHOD_MEMORY_SET) + ",{memory:state}).then(function(){" +
  "      batch.forEach(function(w){w.resolve();});" +
  "      done();" +
  "    },function(err){" +
  "      batch.forEach(function(w){w.reject(err);});" +
  "      done();" +
  "    });" +
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
  "  window.__oretachi={" +
  "    getMemory:getMemory," +
  "    setMemory:setMemory," +
  "    setMemoryKey:patch," +
  "    clearMemory:clearMemory," +
  "    useMemory:useMemory," +
  "    subscribeMemory:subscribe," +
  "    callTool:callTool," +
  "    call:call" +
  "  };" +
  // `state` は setMemory ごとに別オブジェクトへ差し替わるので、スナップショットを
  // 掴ませないようゲッターにする（`oretachi.memory` は常に最新を指す）
  "  Object.defineProperty(window.__oretachi,'memory',{get:getMemory,enumerable:true});" +
  "})();";
