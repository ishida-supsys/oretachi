//! アプリ API: 専用サブウィンドウ（透明・枠なしも可）。想定用途はアシスタントキャラクターの表示。
//!
//! - ラベルは `appinst-<n>-w<k>`（`app_instance::LABEL_PREFIX` の配下）。`capabilities/default.json` の glob には
//!   含まれず、`capabilities/appinst.json`（`appinst-*`）の最小権限だけを持つ。**アプリのコードはシェルの中の
//!   `sandbox="allow-scripts"` iframe の中でだけ動く**（トップレベル文書にならない）。
//! - 呼び出し元インスタンスは、呼んだ webview のラベルから決める。サブウィンドウから呼んでも所有インスタンスの
//!   grant・関門を通る（`app_instance::context_for_label` が親へ解決する）。**追加する全コマンドで
//!   `require_instance_window`**。
//! - 開けるのは manifest の `subWindow` が承認されたアプリだけ。数・大きさの上限と、透明・枠なし・常に前面・
//!   位置の指定は、承認済みの権限セット（grant の `resolved`）だけを見て判定する。アーティファクト JSON は見ない。
//! - インスタンスの停止・アーティファクトの削除ではすべて閉じる。ホットリロードでは維持し、
//!   新しい iframe が再接続する（前提 10）。再起動では復元しない（レジストリは in-memory のみ）。
//! - 透明な領域もクリックを受けうるので、大きさの上限は小さめ（`SUB_WINDOW_DIM_CAP`）。タスクバーには
//!   常に出す（枠なしでも、ユーザーが見つけて閉じられるように）。タイトルには必ずアプリ名を付ける。

use crate::app_grant::AppGrantStore;
use crate::app_manifest::{SubWindowPermission, SUB_WINDOW_DIM_MIN};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

/// 全インスタンスを通したサブウィンドウの上限（webview の数は既知のハングと強相関なので小さく保つ）。
pub const MAX_TOTAL_SUB_WINDOWS: usize = 8;
/// `params`（開いたウィンドウへ渡す JSON）のシリアライズ後の上限。
const MAX_PARAMS_BYTES: usize = 16 * 1024;
const MAX_TITLE_CHARS: usize = 100;
/// 位置の絶対値の上限（論理ピクセル。マルチモニターの負の座標を許しつつ、桁外れの値は弾く）。
const MAX_COORD: f64 = 20000.0;

type Key = (String, String);

#[derive(Debug, Clone)]
struct SubEntry {
    /// 所有インスタンスのラベル（`appinst-<n>`）
    parent: String,
    /// アプリに見せる識別子（`w<k>`）
    id: String,
    key: Key,
    params: serde_json::Value,
    transparent: bool,
    frameless: bool,
    always_on_top: bool,
    width: u32,
    height: u32,
    /// 連番。上限が下がったとき、新しいものから閉じる
    seq: u64,
}

#[derive(Default)]
struct Inner {
    by_label: HashMap<String, SubEntry>,
    seq: u64,
}

impl Inner {
    fn count_for(&self, parent: &str) -> usize {
        self.by_label.values().filter(|e| e.parent == parent).count()
    }

    /// 上限を検査して枠を確保する。ウィンドウを作る前に呼ぶ（作る間に別の呼び出しが上限を破らないように）。
    fn reserve(&mut self, parent: &str, key: &Key, perm: &SubWindowPermission, v: &Validated) -> Result<(String, String), String> {
        if self.count_for(parent) >= perm.max_count as usize {
            return Err(format!(
                "このアプリが同時に開けるサブウィンドウは {} 個までです（承認済みの maxCount）",
                perm.max_count
            ));
        }
        if self.by_label.len() >= MAX_TOTAL_SUB_WINDOWS {
            return Err(format!("サブウィンドウは全体で {} 個までです", MAX_TOTAL_SUB_WINDOWS));
        }
        self.seq += 1;
        let id = format!("w{}", self.seq);
        let label = format!("{}-{}", parent, id);
        self.by_label.insert(
            label.clone(),
            SubEntry {
                parent: parent.to_string(),
                id: id.clone(),
                key: key.clone(),
                params: v.params.clone(),
                transparent: v.transparent,
                frameless: v.frameless,
                always_on_top: v.always_on_top,
                width: v.width,
                height: v.height,
                seq: self.seq,
            },
        );
        Ok((label, id))
    }
}

#[derive(Default)]
pub struct AppSubWindowRegistry(Mutex<Inner>);

// ─── 検査（純粋関数） ─────────────────────────────────────────────────

/// アプリから受ける `open` の指定。省略は「申請の範囲で既定」。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OpenRequest {
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub title: Option<String>,
    pub transparent: Option<bool>,
    pub frameless: Option<bool>,
    pub always_on_top: Option<bool>,
    pub resizable: Option<bool>,
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Validated {
    pub width: u32,
    pub height: u32,
    pub position: Option<(f64, f64)>,
    pub title: Option<String>,
    pub transparent: bool,
    pub frameless: bool,
    pub always_on_top: bool,
    pub resizable: bool,
    pub params: serde_json::Value,
}

fn dimension(name: &str, v: f64, max: u32) -> Result<u32, String> {
    if !v.is_finite() {
        return Err(format!("{} は有限の数値で指定してください", name));
    }
    // 小数は四捨五入する
    let n = v.round();
    if n < SUB_WINDOW_DIM_MIN as f64 || n > max as f64 {
        return Err(format!(
            "{} は {}〜{} の範囲で指定してください（承認済みの上限。指定: {}）",
            name, SUB_WINDOW_DIM_MIN, max, v
        ));
    }
    Ok(n as u32)
}

/// 新しいサイズが承認済みの範囲に収まるか。
pub fn validate_size(perm: &SubWindowPermission, width: f64, height: f64) -> Result<(u32, u32), String> {
    Ok((dimension("width", width, perm.max_width)?, dimension("height", height, perm.max_height)?))
}

fn validate_position(perm: &SubWindowPermission, x: f64, y: f64) -> Result<(f64, f64), String> {
    if !perm.position {
        return Err("位置の指定は承認されていません（manifest の subWindow.position）".to_string());
    }
    if !x.is_finite() || !y.is_finite() || x.abs() > MAX_COORD || y.abs() > MAX_COORD {
        return Err(format!("位置は ±{} 以内の数値で指定してください", MAX_COORD));
    }
    Ok((x.round(), y.round()))
}

/// `open` の指定を、承認済みの権限セット `perm` に照らして検査する。
pub fn validate_open(perm: &SubWindowPermission, req: &OpenRequest) -> Result<Validated, String> {
    let (width, height) = validate_size(
        perm,
        req.width.unwrap_or(perm.max_width as f64),
        req.height.unwrap_or(perm.max_height as f64),
    )?;
    let want = |name: &str, requested: Option<bool>, granted: bool| -> Result<bool, String> {
        match requested {
            // 省略は「申請した見た目をそのまま」ではなく false（必要な窓だけが明示して使う）
            None | Some(false) => Ok(false),
            Some(true) if granted => Ok(true),
            Some(true) => Err(format!("{} は承認されていません（manifest の subWindow.{}）", name, name)),
        }
    };
    let transparent = want("transparent", req.transparent, perm.transparent)?;
    let frameless = want("frameless", req.frameless, perm.frameless)?;
    let always_on_top = want("alwaysOnTop", req.always_on_top, perm.always_on_top)?;
    let position = match (req.x, req.y) {
        (None, None) => None,
        (Some(x), Some(y)) => Some(validate_position(perm, x, y)?),
        _ => return Err("x と y はそろえて指定してください".to_string()),
    };
    let title = match req.title.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(t) => {
            if t.chars().count() > MAX_TITLE_CHARS {
                return Err(format!("title は {} 文字以内にしてください", MAX_TITLE_CHARS));
            }
            if t.chars().any(|c| c.is_control()) {
                return Err("title に制御文字は使えません".to_string());
            }
            Some(t.to_string())
        }
    };
    let params = req.params.clone().unwrap_or(serde_json::Value::Null);
    let size = serde_json::to_string(&params).map_err(|e| e.to_string())?.len();
    if size > MAX_PARAMS_BYTES {
        return Err(format!("params は {} バイト以内にしてください（指定: {}）", MAX_PARAMS_BYTES, size));
    }
    Ok(Validated {
        width,
        height,
        position,
        title,
        transparent,
        frameless,
        always_on_top,
        // 枠なしでは枠のリサイズができないので、既定は固定。必要なら明示する
        resizable: req.resizable.unwrap_or(!frameless),
        params,
    })
}

/// 承認済みの権限セットに、既存のウィンドウ 1 件が収まるか（再承認で権限が狭まったときの判定）。
fn entry_fits(perm: &SubWindowPermission, e: &SubEntry) -> bool {
    (!e.transparent || perm.transparent)
        && (!e.frameless || perm.frameless)
        && (!e.always_on_top || perm.always_on_top)
        && e.width <= perm.max_width
        && e.height <= perm.max_height
}

/// 権限が狭まったときに閉じるべきウィンドウ（ラベル）。収まらないものと、数の超過分（新しいものから）。
fn labels_to_close(perm: Option<&SubWindowPermission>, entries: &[(String, SubEntry)]) -> Vec<String> {
    let Some(perm) = perm else {
        return entries.iter().map(|(l, _)| l.clone()).collect();
    };
    let mut out: Vec<String> = entries.iter().filter(|(_, e)| !entry_fits(perm, e)).map(|(l, _)| l.clone()).collect();
    let mut rest: Vec<&(String, SubEntry)> = entries.iter().filter(|(l, _)| !out.contains(l)).collect();
    rest.sort_by_key(|(_, e)| e.seq);
    if rest.len() > perm.max_count as usize {
        out.extend(rest[perm.max_count as usize..].iter().map(|(l, _)| l.clone()));
    }
    out
}

// ─── レジストリの照会（app_instance から呼ぶ） ─────────────────────────

fn with_inner<T>(app: &AppHandle, f: impl FnOnce(&mut Inner) -> T) -> Option<T> {
    let reg = app.state::<AppSubWindowRegistry>();
    let mut inner = reg.0.lock().ok()?;
    Some(f(&mut inner))
}

/// サブウィンドウのラベルから、所有インスタンスのラベルを引く。サブウィンドウでなければ `None`。
pub fn parent_label(app: &AppHandle, label: &str) -> Option<String> {
    with_inner(app, |i| i.by_label.get(label).map(|e| e.parent.clone())).flatten()
}

/// インスタンスが開いているサブウィンドウのラベル（リロード依頼の宛先）。
pub fn labels_for_instance(app: &AppHandle, parent: &str) -> Vec<String> {
    with_inner(app, |i| {
        let mut v: Vec<(u64, String)> =
            i.by_label.iter().filter(|(_, e)| e.parent == parent).map(|(l, e)| (e.seq, l.clone())).collect();
        v.sort();
        v.into_iter().map(|(_, l)| l).collect()
    })
    .unwrap_or_default()
}

/// インスタンスのウィンドウ（常駐ウィンドウと、所有するサブウィンドウ）すべてへイベントを送る。
/// 宛先はこのインスタンスのウィンドウだけ（他のインスタンスへ漏らさない）。
pub fn emit_to_instance<S: Serialize + Clone>(app: &AppHandle, instance_label: &str, event: &str, payload: S) {
    for target in std::iter::once(instance_label.to_string()).chain(labels_for_instance(app, instance_label)) {
        if let Err(e) = app.emit_to(target.as_str(), event, payload.clone()) {
            log::warn!("[app-subwindow] {} へ {} を送れませんでした: {}", target, event, e);
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubWindowInfo {
    pub id: String,
    pub params: serde_json::Value,
    pub transparent: bool,
    pub frameless: bool,
}

/// シェルの bootstrap に載せる自己情報。サブウィンドウでなければ `None`（JSON では null）。
pub fn info_for_label(app: &AppHandle, label: &str) -> Option<SubWindowInfo> {
    with_inner(app, |i| {
        i.by_label.get(label).map(|e| SubWindowInfo {
            id: e.id.clone(),
            params: e.params.clone(),
            transparent: e.transparent,
            frameless: e.frameless,
        })
    })
    .flatten()
}

fn destroy(app: &AppHandle, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        // destroy は CloseRequested を経ない
        if let Err(e) = w.destroy() {
            log::warn!("[app-subwindow] {} の破棄に失敗: {}", label, e);
        }
    }
}

/// インスタンスの停止（`app_instance::release`）。所有するサブウィンドウをすべて閉じる。
pub fn on_instance_released(app: &AppHandle, parent: &str) {
    let labels = with_inner(app, |i| {
        let labels: Vec<String> = i.by_label.iter().filter(|(_, e)| e.parent == parent).map(|(l, _)| l.clone()).collect();
        for l in &labels {
            i.by_label.remove(l);
        }
        labels
    })
    .unwrap_or_default();
    for l in labels {
        destroy(app, &l);
    }
}

/// 再承認で権限セットが入れ替わった。新しい権限に収まらないウィンドウを閉じる（残りは維持する）。
pub fn on_grant_replaced(app: &AppHandle, parent: &str) {
    let mut entries: Vec<(String, SubEntry)> = with_inner(app, |i| {
        i.by_label.iter().filter(|(_, e)| e.parent == parent).map(|(l, e)| (l.clone(), e.clone())).collect()
    })
    .unwrap_or_default();
    // ユーザーのリサイズで記録とずれているので、実寸（論理ピクセル）で判定する
    for (label, e) in entries.iter_mut() {
        if let Some(w) = app.get_webview_window(label) {
            if let (Ok(size), Ok(scale)) = (w.inner_size(), w.scale_factor()) {
                e.width = (size.width as f64 / scale).round() as u32;
                e.height = (size.height as f64 / scale).round() as u32;
            }
        }
    }
    let Some((_, first)) = entries.first() else { return };
    let perm = app
        .state::<AppGrantStore>()
        .get(&first.key.0, &first.key.1)
        .and_then(|g| g.resolved.sub_window);
    let close = labels_to_close(perm.as_ref(), &entries);
    if close.is_empty() {
        return;
    }
    let removed: Vec<SubEntry> = with_inner(app, |i| close.iter().filter_map(|l| i.by_label.remove(l)).collect()).unwrap_or_default();
    for l in &close {
        destroy(app, l);
    }
    for e in removed {
        crate::app_instance::add_log(
            app,
            &e.key.0,
            &e.key.1,
            "warn",
            format!("再承認で権限が狭まったため、サブウィンドウ {} を閉じました", e.id),
        );
    }
}

/// `WindowEvent::Destroyed` から呼ぶ。登録済みのサブウィンドウなら管理から外し、アプリへ知らせて `true`。
/// インスタンスの停止で閉じたもの（先に管理から外している）は `false` を返し、通知もしない。
pub fn on_window_destroyed(app: &AppHandle, label: &str) -> bool {
    let Some(Some(entry)) = with_inner(app, |i| i.by_label.remove(label)) else { return false };
    crate::app_instance::add_log(app, &entry.key.0, &entry.key.1, "info", format!("サブウィンドウ {} が閉じられました", entry.id));
    emit_to_instance(app, &entry.parent, "app-subwindow-event", serde_json::json!({ "event": "closed", "id": entry.id }));
    true
}

// ─── Tauri コマンド（appinst-* ウィンドウからだけ） ─────────────────────

/// 呼び出し元の身元と、承認済みの `subWindow` 権限。権限が無ければ拒否してデバッグログへ残す。
fn caller(
    window: &tauri::Window,
    app: &AppHandle,
    what: &str,
) -> Result<(crate::app_instance::InstanceContext, SubWindowPermission), String> {
    crate::app_instance::require_instance_window(window)?;
    let ctx = crate::app_instance::context_for_label(app, window.label())?;
    let perm = app
        .state::<AppGrantStore>()
        .get(&ctx.worktree_id, &ctx.artifact_id)
        .and_then(|g| g.resolved.sub_window);
    match perm {
        Some(p) => Ok((ctx, p)),
        None => {
            let e = "このアプリにはサブウィンドウの権限がありません（manifest の subWindow）".to_string();
            crate::app_instance::add_log(app, &ctx.worktree_id, &ctx.artifact_id, "warn", format!("拒否: subWindow.{} — {}", what, e));
            Err(e)
        }
    }
}

fn denied<T>(app: &AppHandle, ctx: &crate::app_instance::InstanceContext, what: &str, e: String) -> Result<T, String> {
    crate::app_instance::add_log(app, &ctx.worktree_id, &ctx.artifact_id, "warn", format!("拒否: subWindow.{} — {}", what, e));
    Err(e)
}

/// 自分（インスタンス）が開いたサブウィンドウのラベル。他のインスタンスのものは引けない。
fn owned_label(app: &AppHandle, ctx: &crate::app_instance::InstanceContext, id: &str) -> Result<String, String> {
    let label = format!("{}-{}", ctx.instance_label, id);
    let ok = with_inner(app, |i| i.by_label.get(&label).map(|e| e.parent == ctx.instance_label).unwrap_or(false))
        .unwrap_or(false);
    if ok {
        Ok(label)
    } else {
        Err(format!("サブウィンドウ '{}' は開いていません", id))
    }
}

#[derive(Debug, Serialize)]
pub struct OpenResult {
    pub id: String,
}

#[tauri::command]
pub async fn app_subwindow_open(
    window: tauri::Window,
    app: AppHandle,
    request: Option<OpenRequest>,
) -> Result<OpenResult, String> {
    let (ctx, perm) = caller(&window, &app, "open")?;
    let req = request.unwrap_or_default();
    let v = match validate_open(&perm, &req) {
        Ok(v) => v,
        Err(e) => return denied(&app, &ctx, "open", e),
    };
    let key: Key = (ctx.worktree_id.clone(), ctx.artifact_id.clone());
    let reserved = with_inner(&app, |i| i.reserve(&ctx.instance_label, &key, &perm, &v))
        .ok_or_else(|| "レジストリのロックが壊れています".to_string())?;
    let (label, id) = match reserved {
        Ok(r) => r,
        Err(e) => return denied(&app, &ctx, "open", e),
    };
    // 枠を確保した後でもう一度、インスタンスが生きているかを見る。`release` は先にインスタンスを外してから
    // `on_instance_released` を呼ぶので、この検査が通れば、以後の停止は `on_instance_released` が掃除する
    if crate::app_instance::context_for_label(&app, &ctx.instance_label).is_err() {
        with_inner(&app, |i| i.by_label.remove(&label));
        return Err("アプリが停止したためサブウィンドウを開きませんでした".to_string());
    }

    let app_name = crate::app_instance::instance_app_name(&app, &ctx);
    // タイトルには必ずアプリ名を付ける（枠なしでもタスクバーで、どのアプリのウィンドウか分かる）
    let title = match &v.title {
        Some(t) => format!("{} — {} (oretachi app)", t, app_name),
        None => format!("{} — oretachi app", app_name),
    };
    let mut builder = tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::App("index.html".into()))
        .title(title)
        .inner_size(v.width as f64, v.height as f64)
        // 最小化させない（WebView2 が最小化でタイマーを間引くため。常駐ウィンドウと同じ理由）
        .minimizable(false)
        .resizable(v.resizable)
        // ユーザーのリサイズでも、承認済みの大きさの上限を超えさせない
        .max_inner_size(perm.max_width as f64, perm.max_height as f64)
        .decorations(!v.frameless)
        .transparent(v.transparent)
        .always_on_top(v.always_on_top)
        .disable_drag_drop_handler();
    if v.transparent {
        // 透明なウィンドウに OS の影を付けると、枠の無い四角が残る
        builder = builder.shadow(false);
    }
    builder = match v.position {
        Some((x, y)) => builder.position(x, y),
        None => builder.center(),
    };
    let built = builder.build();
    if let Err(e) = built {
        with_inner(&app, |i| i.by_label.remove(&label));
        return denied(&app, &ctx, "open", format!("サブウィンドウを作れませんでした: {}", e));
    }
    // 作っている間に、停止された（管理から外されて破棄待ち）か、ユーザーに閉じられたら、孤児にしない
    let still_owned = with_inner(&app, |i| i.by_label.contains_key(&label)).unwrap_or(false);
    if !still_owned {
        destroy(&app, &label);
        return Err("サブウィンドウは開いた直後に閉じられました（アプリの停止かユーザー操作）".to_string());
    }
    crate::app_instance::add_log(
        &app,
        &ctx.worktree_id,
        &ctx.artifact_id,
        "info",
        format!(
            "許可: subWindow.open {}（{}x{}{}{}{}）",
            id,
            v.width,
            v.height,
            if v.transparent { " 透明" } else { "" },
            if v.frameless { " 枠なし" } else { "" },
            if v.always_on_top { " 常に前面" } else { "" },
        ),
    );
    Ok(OpenResult { id })
}

#[tauri::command]
pub async fn app_subwindow_close(window: tauri::Window, app: AppHandle, id: String) -> Result<bool, String> {
    let (ctx, _perm) = caller(&window, &app, "close")?;
    let label = match owned_label(&app, &ctx, &id) {
        Ok(l) => l,
        Err(e) => return denied(&app, &ctx, "close", e),
    };
    // 管理から外してから壊す。アプリには closed の通知は送らない（自分で閉じたので）
    let removed = with_inner(&app, |i| i.by_label.remove(&label)).flatten();
    destroy(&app, &label);
    if removed.is_some() {
        crate::app_instance::add_log(&app, &ctx.worktree_id, &ctx.artifact_id, "info", format!("許可: subWindow.close {}", id));
    }
    Ok(removed.is_some())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListItem {
    pub id: String,
    pub width: u32,
    pub height: u32,
    pub transparent: bool,
    pub frameless: bool,
    pub always_on_top: bool,
    pub params: serde_json::Value,
}

#[tauri::command]
pub async fn app_subwindow_list(window: tauri::Window, app: AppHandle) -> Result<Vec<ListItem>, String> {
    let (ctx, _perm) = caller(&window, &app, "list")?;
    let mut items: Vec<(u64, ListItem)> = with_inner(&app, |i| {
        i.by_label
            .values()
            .filter(|e| e.parent == ctx.instance_label)
            .map(|e| {
                (
                    e.seq,
                    ListItem {
                        id: e.id.clone(),
                        width: e.width,
                        height: e.height,
                        transparent: e.transparent,
                        frameless: e.frameless,
                        always_on_top: e.always_on_top,
                        params: e.params.clone(),
                    },
                )
            })
            .collect()
    })
    .unwrap_or_default();
    items.sort_by_key(|(s, _)| *s);
    Ok(items.into_iter().map(|(_, i)| i).collect())
}

#[tauri::command]
pub async fn app_subwindow_set_position(
    window: tauri::Window,
    app: AppHandle,
    id: String,
    x: f64,
    y: f64,
) -> Result<(), String> {
    let (ctx, perm) = caller(&window, &app, "setPosition")?;
    let label = match owned_label(&app, &ctx, &id) {
        Ok(l) => l,
        Err(e) => return denied(&app, &ctx, "setPosition", e),
    };
    let (x, y) = match validate_position(&perm, x, y) {
        Ok(p) => p,
        Err(e) => return denied(&app, &ctx, "setPosition", e),
    };
    let w = app.get_webview_window(&label).ok_or_else(|| format!("サブウィンドウ '{}' は開いていません", id))?;
    w.set_position(tauri::LogicalPosition::new(x, y)).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn app_subwindow_set_size(
    window: tauri::Window,
    app: AppHandle,
    id: String,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let (ctx, perm) = caller(&window, &app, "setSize")?;
    let label = match owned_label(&app, &ctx, &id) {
        Ok(l) => l,
        Err(e) => return denied(&app, &ctx, "setSize", e),
    };
    let (w_px, h_px) = match validate_size(&perm, width, height) {
        Ok(s) => s,
        Err(e) => return denied(&app, &ctx, "setSize", e),
    };
    let w = app.get_webview_window(&label).ok_or_else(|| format!("サブウィンドウ '{}' は開いていません", id))?;
    w.set_size(tauri::LogicalSize::new(w_px as f64, h_px as f64)).map_err(|e| e.to_string())?;
    with_inner(&app, |i| {
        if let Some(e) = i.by_label.get_mut(&label) {
            e.width = w_px;
            e.height = h_px;
        }
    });
    Ok(())
}

/// 枠なしウィンドウをドラッグで動かす。**呼んだウィンドウ自身**だけを動かす（マウスが押されている
/// ウィンドウから呼ぶ必要があるため、id は取らない）。位置の権限（`subWindow.position`）が要る。
#[tauri::command]
pub async fn app_subwindow_start_drag(window: tauri::Window, app: AppHandle) -> Result<(), String> {
    let (ctx, perm) = caller(&window, &app, "startDrag")?;
    if !perm.position {
        return denied(&app, &ctx, "startDrag", "位置の指定は承認されていません（manifest の subWindow.position）".to_string());
    }
    if parent_label(&app, window.label()).is_none() {
        return denied(&app, &ctx, "startDrag", "サブウィンドウからだけ呼べます".to_string());
    }
    window.start_dragging().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perm() -> SubWindowPermission {
        SubWindowPermission {
            max_count: 2,
            max_width: 300,
            max_height: 400,
            transparent: false,
            frameless: false,
            always_on_top: false,
            position: false,
        }
    }

    fn full() -> SubWindowPermission {
        SubWindowPermission { transparent: true, frameless: true, always_on_top: true, position: true, ..perm() }
    }

    fn req(json: serde_json::Value) -> OpenRequest {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn defaults_to_the_approved_maximum_size_and_plain_window() {
        let v = validate_open(&perm(), &OpenRequest::default()).unwrap();
        assert_eq!((v.width, v.height), (300, 400));
        assert!(!v.transparent && !v.frameless && !v.always_on_top);
        assert!(v.position.is_none() && v.resizable);
        assert_eq!(v.params, serde_json::Value::Null);
    }

    #[test]
    fn size_is_bounded_by_the_approved_limits() {
        let p = perm();
        assert!(validate_open(&p, &req(serde_json::json!({ "width": 301 }))).is_err());
        assert!(validate_open(&p, &req(serde_json::json!({ "height": 401 }))).is_err());
        assert!(validate_open(&p, &req(serde_json::json!({ "width": SUB_WINDOW_DIM_MIN - 1 }))).is_err());
        assert!(validate_open(&p, &req(serde_json::json!({ "width": 300, "height": 400 }))).is_ok());
        assert_eq!(validate_size(&p, 100.4, 200.6).unwrap(), (100, 201));
        assert!(validate_size(&p, f64::NAN, 200.0).is_err());
        assert!(validate_size(&p, f64::INFINITY, 200.0).is_err());
    }

    #[test]
    fn appearance_requires_the_matching_grant() {
        let p = perm();
        for key in ["transparent", "frameless", "alwaysOnTop"] {
            let e = validate_open(&p, &req(serde_json::json!({ key: true }))).unwrap_err();
            assert!(e.contains(key), "{key}: {e}");
            // false の明示は常に通る
            assert!(validate_open(&p, &req(serde_json::json!({ key: false }))).is_ok());
        }
        let v = validate_open(&full(), &req(serde_json::json!({ "transparent": true, "frameless": true, "alwaysOnTop": true }))).unwrap();
        assert!(v.transparent && v.frameless && v.always_on_top);
        // 枠なしは既定で固定サイズ
        assert!(!v.resizable);
    }

    #[test]
    fn position_requires_the_grant_and_both_coordinates() {
        assert!(validate_open(&perm(), &req(serde_json::json!({ "x": 1, "y": 2 }))).is_err());
        assert!(validate_open(&full(), &req(serde_json::json!({ "x": 1 }))).is_err());
        let v = validate_open(&full(), &req(serde_json::json!({ "x": -10.4, "y": 200 }))).unwrap();
        assert_eq!(v.position, Some((-10.0, 200.0)));
        assert!(validate_open(&full(), &req(serde_json::json!({ "x": 1e9, "y": 0 }))).is_err());
        assert!(validate_position(&perm(), 0.0, 0.0).is_err());
    }

    #[test]
    fn title_and_params_are_bounded() {
        let p = perm();
        assert!(validate_open(&p, &req(serde_json::json!({ "title": "a".repeat(101) }))).is_err());
        assert!(validate_open(&p, &req(serde_json::json!({ "title": "a\nb" }))).is_err());
        assert_eq!(validate_open(&p, &req(serde_json::json!({ "title": "  hi  " }))).unwrap().title.as_deref(), Some("hi"));
        assert!(validate_open(&p, &req(serde_json::json!({ "title": "  " }))).unwrap().title.is_none());
        let big = "x".repeat(MAX_PARAMS_BYTES);
        assert!(validate_open(&p, &req(serde_json::json!({ "params": big }))).is_err());
        assert!(validate_open(&p, &req(serde_json::json!({ "params": { "a": 1 } }))).is_ok());
    }

    fn key() -> Key {
        ("wt".to_string(), "art".to_string())
    }

    #[test]
    fn count_is_enforced_per_instance_and_globally() {
        let mut inner = Inner::default();
        let v = validate_open(&perm(), &OpenRequest::default()).unwrap();
        let (l1, id1) = inner.reserve("appinst-1", &key(), &perm(), &v).unwrap();
        assert_eq!(l1, format!("appinst-1-{}", id1));
        inner.reserve("appinst-1", &key(), &perm(), &v).unwrap();
        // maxCount = 2
        assert!(inner.reserve("appinst-1", &key(), &perm(), &v).is_err());
        // 別インスタンスは別枠
        inner.reserve("appinst-2", &key(), &perm(), &v).unwrap();
        // 全体の上限
        let mut n = 0;
        for inst in 3..20 {
            let p = format!("appinst-{}", inst);
            while inner.reserve(&p, &key(), &perm(), &v).is_ok() {
                n += 1;
            }
        }
        assert_eq!(inner.by_label.len(), MAX_TOTAL_SUB_WINDOWS);
        assert!(n > 0);
    }

    #[test]
    fn ids_are_never_reused_after_removal() {
        let mut inner = Inner::default();
        let v = validate_open(&perm(), &OpenRequest::default()).unwrap();
        let (l1, id1) = inner.reserve("appinst-1", &key(), &perm(), &v).unwrap();
        inner.by_label.remove(&l1);
        let (_, id2) = inner.reserve("appinst-1", &key(), &perm(), &v).unwrap();
        assert_ne!(id1, id2);
    }

    fn entries(perm: &SubWindowPermission, n: usize, tweak: impl Fn(usize, &mut SubEntry)) -> Vec<(String, SubEntry)> {
        let mut inner = Inner::default();
        let v = validate_open(&full(), &OpenRequest::default()).unwrap();
        let big = SubWindowPermission { max_count: 4, ..*perm };
        for _ in 0..n {
            inner.reserve("appinst-1", &key(), &big, &v).unwrap();
        }
        let mut out: Vec<(String, SubEntry)> = inner.by_label.into_iter().collect();
        out.sort_by_key(|(_, e)| e.seq);
        for (i, (_, e)) in out.iter_mut().enumerate() {
            tweak(i, e);
        }
        out
    }

    #[test]
    fn narrowing_closes_only_what_no_longer_fits() {
        let es = entries(&full(), 3, |i, e| e.transparent = i == 1);
        // 透明の権限が外れた → 透明な 1 件だけ閉じる
        let narrowed = SubWindowPermission { transparent: false, max_count: 4, ..full() };
        assert_eq!(labels_to_close(Some(&narrowed), &es), vec![es[1].0.clone()]);
        // 何も変わらない
        assert!(labels_to_close(Some(&SubWindowPermission { max_count: 4, ..full() }), &es).is_empty());
        // 権限そのものが無くなった → すべて
        assert_eq!(labels_to_close(None, &es).len(), 3);
        // 大きさの上限が下がった
        let small = SubWindowPermission { max_width: 100, max_count: 4, ..full() };
        assert_eq!(labels_to_close(Some(&small), &es).len(), 3);
    }

    #[test]
    fn narrowing_the_count_closes_the_newest_first() {
        let es = entries(&full(), 3, |_, e| {
            e.transparent = false;
            e.frameless = false;
            e.always_on_top = false;
        });
        let one = SubWindowPermission { max_count: 1, ..full() };
        let close = labels_to_close(Some(&one), &es);
        assert_eq!(close, vec![es[1].0.clone(), es[2].0.clone()]);
    }
}
