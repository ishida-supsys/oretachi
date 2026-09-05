
// このファイルはそのまま利用（カスタマイズ不要）
// 停止条件の読み出し・集計・フェーズ判定を1か所に集約する。
// TaskNode / DependencyEdge / entry-point はすべてここ経由で停止条件を扱うこと
// （生の `entity.stopConditions` を直接読むと旧スキーマのフォールバックが抜ける）。

// 停止条件の3フェーズ配色。
//   pending  — まだそのタスク/遷移に到達していない(灰)
//   active   — 今まさに人の判定を待っている(橙)
//   cleared  — 全条件クリア済み(緑)
const STOP_PHASE_COLORS = {
  pending: '#6c7086',
  active: '#fab387',
  cleared: '#a6e3a1',
};

// task / dep から停止条件の配列を取り出す。
// 旧スキーマ(stopConditions 未定義 かつ requiresUserConfirmation:true)は
// confirmationNote を未クリア1件として扱う。
function getStopConditions(entity) {
  if (!entity) return [];
  if (Array.isArray(entity.stopConditions)) return entity.stopConditions;
  if (entity.requiresUserConfirmation) {
    return [{
      id: (entity.id || 'legacy') + '-legacy',
      text: entity.confirmationNote || 'ユーザー確認が必要です',
      checked: false,
    }];
  }
  return [];
}

// { total, done, open, hasOpenStop } を返す。
// requiresUserConfirmation は「未クリアの停止条件があるか」の導出値 = hasOpenStop。
function stopStats(entity) {
  const list = getStopConditions(entity);
  const done = list.filter(sc => sc.checked).length;
  return { total: list.length, done, open: list.length - done, hasOpenStop: done < list.length };
}

// タスクの停止条件が「到達済み」か。未着手・ブロック中はまだ子が動いていないので pending 扱い。
function isTaskActive(task) {
  return !!task && task.status !== 'not_started' && task.status !== 'blocked';
}

// エッジの停止条件が「到達済み」か。遷移元が done = 親が次タスクを起動しようとする時点。
function isEdgeActive(fromTask) {
  return !!fromTask && fromTask.status === 'done';
}

// 'pending' | 'active' | 'cleared' を返す。停止条件が無い場合は null。
// active は呼び出し側が isTaskActive / isEdgeActive で判定して渡す。
function stopPhase(entity, active) {
  const list = getStopConditions(entity);
  if (list.length === 0) return null;
  if (list.every(sc => sc.checked)) return 'cleared';
  return active ? 'active' : 'pending';
}

exports.STOP_PHASE_COLORS = STOP_PHASE_COLORS;
exports.getStopConditions = getStopConditions;
exports.stopStats = stopStats;
exports.isTaskActive = isTaskActive;
exports.isEdgeActive = isEdgeActive;
exports.stopPhase = stopPhase;
