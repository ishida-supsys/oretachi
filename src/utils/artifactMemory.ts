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

/** メモリー全体を保存する。params は `{ memory: object }` */
export const ARTIFACT_BRIDGE_METHOD_MEMORY_SET = "memory.set";

/** 応答が返らないまま Promise が残り続けないようにするタイムアウト（iframe 側） */
const BRIDGE_TIMEOUT_MS = 10000;
/** 1 文字入力ごとに保存が飛ばないようにする debounce（iframe 側） */
const MEMORY_FLUSH_DEBOUNCE_MS = 400;

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
 * iframe 内に注入するブリッジ本体。`window.__oretachi` を定義し、
 * `reactArtifactSrcdoc.ts` の makeRequire が `require('oretachi')` として返す。
 *
 * 初期値は同期で読めなければ初回レンダリングでフォームを復元できないため、
 * `_source` / `_modules` と同じく `_memory` textarea から同期で読む。
 * 書き込みだけが postMessage + debounce になる。
 */
export const ARTIFACT_BRIDGE_JS =
  "(function(){" +
  // ── リクエスト/レスポンスの土台（メモリー以外の method も後からここに乗る）──
  "  var pending={};" +
  "  var seq=0;" +
  "  window.addEventListener('message',function(e){" +
  "    var d=e.data;" +
  "    if(!d||typeof d!=='object'||d[" + JSON.stringify(ARTIFACT_BRIDGE_RESULT_MARKER) + "]!==true)return;" +
  "    var p=pending[d.requestId];" +
  "    if(!p)return;" +
  "    delete pending[d.requestId];" +
  "    clearTimeout(p.timer);" +
  "    if(d.ok)p.resolve(d.result);" +
  "    else p.reject(new Error(String(d.error||'oretachi bridge error')));" +
  "  });" +
  "  function call(method,params){" +
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
  // debounce 中の setMemory はまとめて 1 回の保存にし、その 1 回の結果を全員へ返す
  "  var flushTimer=null;" +
  "  var waiters=[];" +
  "  function flush(){" +
  "    flushTimer=null;" +
  "    var batch=waiters;" +
  "    waiters=[];" +
  "    call(" + JSON.stringify(ARTIFACT_BRIDGE_METHOD_MEMORY_SET) + ",{memory:state}).then(function(){" +
  "      batch.forEach(function(w){w.resolve();});" +
  "    },function(err){" +
  "      batch.forEach(function(w){w.reject(err);});" +
  "    });" +
  "  }" +
  "  function schedule(){" +
  "    return new Promise(function(resolve,reject){" +
  "      waiters.push({resolve:resolve,reject:reject});" +
  "      if(flushTimer!==null)clearTimeout(flushTimer);" +
  "      flushTimer=setTimeout(flush," + MEMORY_FLUSH_DEBOUNCE_MS + ");" +
  "    });" +
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
  "    initialRef.current=initialValue;" +
  "    R.useEffect(function(){" +
  "      setValue(read());" +
  "      return subscribe(function(next){" +
  "        var v=next[key]!==undefined?next[key]:initialRef.current;" +
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
  "  window.__oretachi={" +
  "    getMemory:getMemory," +
  "    setMemory:setMemory," +
  "    setMemoryKey:patch," +
  "    clearMemory:clearMemory," +
  "    useMemory:useMemory," +
  "    subscribeMemory:subscribe," +
  "    call:call" +
  "  };" +
  // `state` は setMemory ごとに別オブジェクトへ差し替わるので、スナップショットを
  // 掴ませないようゲッターにする（`oretachi.memory` は常に最新を指す）
  "  Object.defineProperty(window.__oretachi,'memory',{get:getMemory,enumerable:true});" +
  "})();";
