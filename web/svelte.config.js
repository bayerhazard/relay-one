import adapter from "@sveltejs/adapter-static";

/** @type {import('@sveltejs/kit').Config} */
const config = {
  kit: {
    adapter: adapter({
      pages: "build",
      assets: "build",
      fallback: "index.html",
      precompress: true,
      strict: true,
    }),
    paths: {
      base: process.env.RELAY_BASE_PATH ?? "",
    },
    alias: {
      $lib: "src/lib",
    },
  },
};

export default config;
