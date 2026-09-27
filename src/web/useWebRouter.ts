import { onBeforeUnmount, onMounted, ref, type Ref } from "vue";
import { buildWebPath, parseWebPath, type WebRoute } from "../utils/webRoute";

export interface WebRouter {
  route: Ref<WebRoute>;
  /** 新しい履歴エントリとして遷移する（同じパスなら何もしない） */
  push(route: WebRoute): void;
  /** 現在の履歴エントリを差し替える（同じパスなら何もしない） */
  replace(route: WebRoute): void;
}

/**
 * History API の薄いラッパー。ルートが3種類しか無いため vue-router は使わない。
 * ブラウザの戻る/進む（popstate）を購読して `route` を追従させる。
 */
export function useWebRouter(): WebRouter {
  const route = ref<WebRoute>(parseWebPath(window.location.pathname)) as Ref<WebRoute>;

  function onPopState() {
    route.value = parseWebPath(window.location.pathname);
  }

  onMounted(() => window.addEventListener("popstate", onPopState));
  onBeforeUnmount(() => window.removeEventListener("popstate", onPopState));

  function push(next: WebRoute) {
    const path = buildWebPath(next);
    if (path !== window.location.pathname) {
      window.history.pushState(null, "", path);
    }
    route.value = next;
  }

  function replace(next: WebRoute) {
    const path = buildWebPath(next);
    if (path !== window.location.pathname) {
      window.history.replaceState(null, "", path);
    }
    route.value = next;
  }

  return { route, push, replace };
}
