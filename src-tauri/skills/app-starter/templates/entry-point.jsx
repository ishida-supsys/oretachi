// アプリ形式アーティファクトの最小サンプル。manifest（../app.json）で宣言した
// MCP ツールだけが呼べる。宣言していないツールは、ユーザーが承認していても拒否される。
//
// CUSTOMIZE: ここから先を作りたいアプリに置き換える。呼べるツールを増やすときは
// app.json の permissions.mcp.tools に足す（権限が変わるので、起動中なら再承認のダイアログが出る）。
import React, { useState } from 'react';
import { callTool } from 'oretachi';

const errText = (e) => String((e && e.message) || e);

export default function App() {
  const [rows, setRows] = useState(null);
  const [message, setMessage] = useState('');
  const [busy, setBusy] = useState(false);

  async function refresh() {
    setBusy(true);
    setMessage('');
    try {
      // 宣言済みのツール。戻り値は JSON.parse 済み（パースできなければ文字列）
      setRows(await callTool('oretachi_list_worktree_notifications', {}));
    } catch (e) {
      setMessage('読み込みに失敗: ' + errText(e));
    } finally {
      setBusy(false);
    }
  }

  async function sendTest() {
    setBusy(true);
    setMessage('');
    try {
      // 宛先は常に自分のワークツリーへ固定される。本文には出自が自動で前置される
      await callTool('notify_worktree', { kind: 'general', body: 'アプリからのテスト通知' });
      setMessage('テスト通知を送りました');
    } catch (e) {
      setMessage('送信に失敗: ' + errText(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="min-h-screen bg-slate-900 text-slate-100 p-6 space-y-4">
      <h1 className="text-xl font-semibold">Notification Viewer</h1>
      <div className="flex gap-2">
        <button
          className="px-3 py-1.5 rounded bg-sky-500 text-slate-900 font-medium disabled:opacity-50"
          disabled={busy}
          onClick={refresh}
        >
          通知を読み込む
        </button>
        <button
          className="px-3 py-1.5 rounded bg-slate-700 disabled:opacity-50"
          disabled={busy}
          onClick={sendTest}
        >
          テスト通知を送る
        </button>
      </div>
      {message && <p className="text-sm text-amber-300">{message}</p>}
      {rows !== null && (
        <pre className="text-xs bg-slate-800 rounded p-3 overflow-auto max-h-[60vh]">
          {typeof rows === 'string' ? rows : JSON.stringify(rows, null, 2)}
        </pre>
      )}
    </div>
  );
}
