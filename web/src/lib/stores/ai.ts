import { writable } from "svelte/store";

interface AIStreamState {
  active: boolean;
  content: string;
  error: string | null;
}

function createAIStore() {
  const { subscribe, update } = writable<AIStreamState>({
    active: false,
    content: "",
    error: null,
  });

  return {
    subscribe,
    startStreaming: () =>
      update((s) => ({ ...s, active: true, content: "", error: null })),
    appendContent: (token: string) =>
      update((s) => ({ ...s, content: s.content + token })),
    endStreaming: () => update((s) => ({ ...s, active: false })),
    setError: (error: string) =>
      update((s) => ({ ...s, active: false, error })),
    reset: () =>
      update(() => ({ active: false, content: "", error: null })),
  };
}

export const aiStreaming = createAIStore();
