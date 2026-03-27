import { type Ref, onUnmounted } from "vue";

interface UseInfiniteScrollOptions {
  hasMore: Ref<boolean>;
  isLoading: Ref<boolean>;
  rootSelector?: string;
  rootMargin?: string;
}

export function useInfiniteScroll(
  sentinelRef: Ref<HTMLElement | null>,
  loadMore: () => Promise<void>,
  options: UseInfiniteScrollOptions
) {
  const { hasMore, isLoading, rootSelector = ".home-content", rootMargin = "200px" } = options;
  let observer: IntersectionObserver | null = null;

  function setup() {
    teardown();
    if (!sentinelRef.value) return;
    const root = sentinelRef.value.closest(rootSelector);
    observer = new IntersectionObserver(
      (entries) => {
        if (entries[0].isIntersecting && hasMore.value && !isLoading.value) {
          loadMore();
        }
      },
      { root: root as Element, rootMargin }
    );
    observer.observe(sentinelRef.value);
  }

  function teardown() {
    observer?.disconnect();
    observer = null;
  }

  onUnmounted(teardown);

  return { setup, teardown };
}
