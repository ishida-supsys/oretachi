import { ref, onMounted, onUnmounted } from "vue";

export function useWindowFocus() {
  const isWindowFocused = ref(true);

  function handleWindowFocus() {
    isWindowFocused.value = true;
  }

  function handleWindowBlur() {
    isWindowFocused.value = false;
  }

  onMounted(() => {
    isWindowFocused.value = document.hasFocus();
    window.addEventListener("focus", handleWindowFocus);
    window.addEventListener("blur", handleWindowBlur);
  });

  onUnmounted(() => {
    window.removeEventListener("focus", handleWindowFocus);
    window.removeEventListener("blur", handleWindowBlur);
  });

  return { isWindowFocused };
}
