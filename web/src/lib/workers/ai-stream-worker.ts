interface StreamRequest {
  url: string;
  apiKey: string;
  model: string;
  systemPrompt: string;
  userPrompt: string;
  temperature: number;
}

let abortController: AbortController | null = null;

self.onmessage = async (event: MessageEvent<StreamRequest | "abort">) => {
  if (event.data === "abort") {
    abortController?.abort();
    abortController = null;
    return;
  }

  const { url, apiKey, model, systemPrompt, userPrompt, temperature } = event.data;
  abortController = new AbortController();
  const signal = abortController.signal;

  try {
    const apiUrl = `${url.replace(/\/$/, "")}/chat/completions`;

    const response = await fetch(apiUrl, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${apiKey}`,
      },
      body: JSON.stringify({
        model,
        messages: [
          { role: "system", content: systemPrompt },
          { role: "user", content: userPrompt },
        ],
        stream: true,
        temperature,
        max_tokens: 4096,
      }),
      signal,
    });

    if (!response.ok) {
      self.postMessage({ type: "error", error: `HTTP ${response.status}` });
      return;
    }

    const reader = response.body?.getReader();
    if (!reader) {
      self.postMessage({ type: "error", error: "Kein Stream-Body" });
      return;
    }

    const decoder = new TextDecoder();
    let buffer = "";

    while (true) {
      const { done, value } = await reader.read();
      if (done) break;

      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split("\n");
      buffer = lines.pop() ?? "";

      for (const line of lines) {
        const trimmed = line.trim();
        if (!trimmed || trimmed === "data: [DONE]") continue;

        if (trimmed.startsWith("data: ")) {
          try {
            const json = JSON.parse(trimmed.slice(6));
            const content = json.choices?.[0]?.delta?.content;
            if (content) {
              self.postMessage({ type: "token", token: content });
            }
          } catch {
            // ignore malformed chunks
          }
        }
      }
    }

    self.postMessage({ type: "done" });
  } catch (err: unknown) {
    const error = err instanceof Error ? err : new Error(String(err));
    if (error.name === "AbortError") {
      self.postMessage({ type: "aborted" });
    } else {
      self.postMessage({ type: "error", error: error.message });
    }
  }
};
