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

export function buildReactSrcdoc(
  react: string,
  reactDom: string,
  babel: string,
  content: string
): string {
  // Vue SFC パーサーに誤認識されないよう文字列連結でタグを組み立てる
  const openTag = (js: string) => "<script>" + js + "<" + "/script>";

  const styles = `
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


  const runtimeJs =
    "(function(){" +
    "var errEl=document.getElementById('error-display');" +
    "function showError(e){" +
    "  errEl.style.display='block';" +
    "  errEl.textContent=(e instanceof Error)?(e.message+(e.stack?'\\n\\n'+e.stack:'')):String(e);" +
    "}" +
    "window.onerror=function(msg,src,line,col,err){showError(err||new Error(msg));return true;};" +
    "try{" +
    "  var source=document.getElementById('_source').value;" +
    "  var transformed=Babel.transform(source,{presets:['env','react','typescript'],filename:'artifact.tsx'}).code;" +
    "  var exports={};" +
    "  var module={exports:exports};" +
    "  var require=function(name){" +
    "    var libs={'react':React,'react-dom':ReactDOM,'react-dom/client':ReactDOM,'react/jsx-runtime':React};" +
    "    if(libs[name]!==undefined)return libs[name];" +
    "    throw new Error('Module not found: '+name);" +
    "  };" +
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

  return (
    "<!DOCTYPE html>\n<html>\n<head>\n" +
    '<meta charset="utf-8" />\n' +
    '<meta name="viewport" content="width=device-width, initial-scale=1" />\n' +
    // connect-src 'none' で fetch/XHR/WebSocket による外部通信をブロック
    '<meta http-equiv="Content-Security-Policy" content="default-src \'none\'; script-src \'unsafe-inline\' \'unsafe-eval\'; style-src \'unsafe-inline\';" />\n' +
    "<style>" + styles + "</style>\n" +
    openTag(react) + "\n" +
    openTag(reactDom) + "\n" +
    openTag(babel) + "\n" +
    "</head>\n<body>\n" +
    '<div id="root"></div>\n' +
    '<div id="error-display" class="error-overlay" style="display:none"></div>\n' +
    '<textarea id="_source" style="display:none">' + htmlEscape(content) + "</textarea>\n" +
    openTag(runtimeJs) + "\n" +
    "</body>\n</html>"
  );
}
