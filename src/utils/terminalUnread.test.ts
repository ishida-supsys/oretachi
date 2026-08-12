import { describe, it, expect } from "vitest";
import { collectUnreadByTab } from "./terminalUnread";

describe("collectUnreadByTab", () => {
  it("session_id キーを フロント terminalId キーへ詰め替える", () => {
    const unread = new Map([
      [101, 2],
      [102, 5],
    ]);
    const entries = new Map([
      [1, { sessionId: 101 }],
      [2, { sessionId: 102 }],
    ]);
    expect([...collectUnreadByTab(unread, entries)]).toEqual([
      [1, 2],
      [2, 5],
    ]);
  });

  it("未読が無いセッションのタブは入れない (FramePane が falsy で出し分けるため)", () => {
    const unread = new Map([[101, 3]]);
    const entries = new Map([
      [1, { sessionId: 101 }],
      [2, { sessionId: 999 }],
    ]);
    const result = collectUnreadByTab(unread, entries);
    expect(result.has(2)).toBe(false);
    expect(result.get(1)).toBe(3);
  });

  it("sessionId が未確定 / エントリが無いタブを飛ばす", () => {
    const unread = new Map([[101, 1]]);
    const entries: Array<[number, { sessionId?: number | null } | undefined | null]> = [
      [1, undefined],
      [2, null],
      [3, {}],
      [4, { sessionId: null }],
      [5, { sessionId: 101 }],
    ];
    expect([...collectUnreadByTab(unread, entries)]).toEqual([[5, 1]]);
  });

  it("未読ゼロなら累積先をそのまま返す", () => {
    const into = new Map([[9, 1]]);
    const result = collectUnreadByTab(new Map(), [[1, { sessionId: 101 }]], into);
    expect(result).toBe(into);
    expect([...result]).toEqual([[9, 1]]);
  });

  it("複数のワークツリーバンドルを 1 つのマップへ累積できる", () => {
    const unread = new Map([
      [101, 1],
      [201, 4],
    ]);
    const result = new Map<number, number>();
    collectUnreadByTab(unread, new Map([[1, { sessionId: 101 }]]), result);
    collectUnreadByTab(unread, new Map([[7, { sessionId: 201 }]]), result);
    expect([...result]).toEqual([
      [1, 1],
      [7, 4],
    ]);
  });
});
