import { writable } from "svelte/store";

export const isOnline = writable<boolean>(
  typeof navigator !== "undefined" ? navigator.onLine : true
);

let initialized = false;

export function initOnlineListener(): void {
  if (initialized || typeof window === "undefined") return;
  initialized = true;

  window.addEventListener("online", () => {
    isOnline.set(true);
  });
  window.addEventListener("offline", () => {
    isOnline.set(false);
  });
}
