/**
 * React アーティファクト用の srcdoc HTML を生成するユーティリティ。
 * Vue SFC の外に置くことで、スクリプトタグ文字列が SFC パーサーと干渉しない。
 */

function htmlEscape(str: string): string {
  return str
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

// Vue SFC パーサーに誤認識されないよう文字列連結でタグを組み立てる
// ベンダーJS内に </script> リテラルが含まれる場合に備えてエスケープする
function escapeScript(js: string): string {
  return js.replace(/<\/script>/gi, "<\\/script>");
}
function openTag(js: string): string {
  return "<script>" + escapeScript(js) + "<" + "/script>";
}

const STYLES = `
  *, *::before, *::after { box-sizing: border-box; }
  body { margin: 0; font-family: system-ui, -apple-system, sans-serif; background: #fff; }
  #root { min-height: 100vh; }
  .error-overlay {
    color: #f38ba8;
    background: rgba(243, 139, 168, 0.08);
    border: 1px solid rgba(243, 139, 168, 0.3);
    border-radius: 6px; padding: 16px; margin: 16px;
    font-family: monospace; font-size: 13px;
    white-space: pre-wrap; word-break: break-word;
  }
`;

const RUNTIME_JS =
  "(function(){" +
  "var errEl=document.getElementById('error-display');" +
  "function showError(e){" +
  "  errEl.style.display='block';" +
  "  errEl.textContent=(e instanceof Error)?(e.message+(e.stack?'\\n\\n'+e.stack:'')):String(e);" +
  "}" +
  "window.onerror=function(msg,src,line,col,err){showError(err||new Error(msg));return true;};" +
  "try{" +
  "  var moduleSources=JSON.parse(document.getElementById('_modules').value||'{}');" +
  "  var moduleCache={};" +
  "  function makeRequire(){" +
  "    var libs={'react':React,'react-dom':ReactDOM,'react-dom/client':ReactDOM,'react/jsx-runtime':React};" +
  "    function req(name){" +
  "      if(libs[name]!==undefined)return libs[name];" +
  "      var key=name.replace(/^\\.\\//, '');" +
  "      if(moduleSources[key]===undefined)throw new Error('Module not found: '+name);" +
  "      if(moduleCache[key])return moduleCache[key].exports;" +
  "      var mod={exports:{}};" +
  "      moduleCache[key]=mod;" +
  "      var code=Babel.transform(moduleSources[key],{presets:['env','react','typescript'],filename:key+'.tsx'}).code;" +
  "      var fn=new Function('React','ReactDOM','exports','module','require',code);" +
  "      fn(React,ReactDOM,mod.exports,mod,req);" +
  "      return mod.exports;" +
  "    }" +
  "    return req;" +
  "  }" +
  "  var require=makeRequire();" +
  "  var source=document.getElementById('_source').value;" +
  "  var transformed=Babel.transform(source,{presets:['env','react','typescript'],filename:'artifact.tsx'}).code;" +
  "  var exports={};" +
  "  var module={exports:exports};" +
  "  var fn=new Function('React','ReactDOM','exports','module','require',transformed);" +
  "  fn(React,ReactDOM,exports,module,require);" +
  "  var Component=exports['default']||module.exports['default']||module.exports;" +
  "  if(!Component||typeof Component!=='function'){" +
  "    for(var k in exports){if(typeof exports[k]==='function'){Component=exports[k];break;}}" +
  "  }" +
  "  if(!Component||typeof Component!=='function'){" +
  "    showError(new Error('React component not found. Use default export.'));return;" +
  "  }" +
  "  var EB=(function(){" +
  "    function EB(p){React.Component.call(this,p);this.state={error:null};}" +
  "    EB.prototype=Object.create(React.Component.prototype);" +
  "    EB.prototype.constructor=EB;" +
  "    EB.getDerivedStateFromError=function(e){return{error:e};};" +
  "    EB.prototype.render=function(){" +
  "      if(this.state.error)return React.createElement('div',{className:'error-overlay'},'Runtime error: '+String(this.state.error));" +
  "      return this.props.children;" +
  "    };" +
  "    return EB;" +
  "  })();" +
  "  var root=ReactDOM.createRoot(document.getElementById('root'));" +
  "  root.render(React.createElement(EB,null,React.createElement(Component)));" +
  "}catch(e){showError(e);}" +
  "})();";

/**
 * ベンダースクリプト（React/ReactDOM/Babel/Tailwind）から静的な <head> 部分を構築する。
 * content が変わっても再計算不要なため、呼び出し側でキャッシュすること。
 */
export function buildVendorHead(react: string, reactDom: string, babel: string, tailwind: string): string {
  return (
    "<!DOCTYPE html>\n<html>\n<head>\n" +
    '<meta charset="utf-8" />\n' +
    '<meta name="viewport" content="width=device-width, initial-scale=1" />\n' +
    // connect-src 'none' で fetch/XHR/WebSocket による外部通信をブロック
    // unsafe-eval は Babel.transform() と new Function() に必要な設計上の要件。
    // sandbox="allow-scripts" のみ（allow-same-origin なし）により、
    // 親ウィンドウ・Cookie・localStorage へのアクセスは遮断されている。
    '<meta http-equiv="Content-Security-Policy" content="default-src \'none\'; script-src \'unsafe-inline\' \'unsafe-eval\'; style-src \'unsafe-inline\'; img-src data: blob:;" />\n' +
    "<style>" + STYLES + "</style>\n" +
    // @tailwindcss/browser の初期化エントリポイント
    '<style type="text/tailwindcss">@import "tailwindcss";</style>\n' +
    openTag(react) + "\n" +
    openTag(reactDom) + "\n" +
    openTag(babel) + "\n" +
    openTag(tailwind) + "\n" +
    "</head>\n"
  );
}

/**
 * vendorHead（buildVendorHead の結果）と content を組み合わせて完全な srcdoc を生成する。
 * content が変わるたびに呼ばれるが、ベンダー部分は含まない。
 * modules が指定された場合、各モジュールは require() で解決可能になる。
 */
export function buildReactSrcdoc(vendorHead: string, content: string, modules?: Record<string, string>): string {
  const modulesJson = modules && Object.keys(modules).length > 0
    ? htmlEscape(JSON.stringify(modules))
    : "{}";
  return (
    vendorHead +
    "<body>\n" +
    '<div id="root"></div>\n' +
    '<div id="error-display" class="error-overlay" style="display:none"></div>\n' +
    '<textarea id="_source" style="display:none">' + htmlEscape(content) + "</textarea>\n" +
    '<textarea id="_modules" style="display:none">' + modulesJson + "</textarea>\n" +
    openTag(RUNTIME_JS) + "\n" +
    "</body>\n</html>"
  );
}
