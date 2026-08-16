import { afterEach } from "vitest";

Object.defineProperty(window, "matchMedia", {
  writable: true,
  value: () => ({ matches: false, addEventListener() {}, removeEventListener() {} }),
});

afterEach(() => {
  document.body.innerHTML = "";
});
