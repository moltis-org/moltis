import preact from "@preact/preset-vite";
import { resolve } from "path";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [preact()],
  root: ".",
  resolve: {
    alias: {
      "@": resolve(__dirname, "src"),
    },
  },
  build: {
    outDir: resolve(__dirname, "../src/assets/dist"),
    emptyOutDir: true,
    sourcemap: false,
    minify: false,
    rollupOptions: {
      input: {
        main: resolve(__dirname, "src/app.tsx"),
        login: resolve(__dirname, "src/login-app.tsx"),
        onboarding: resolve(__dirname, "src/onboarding-app.tsx"),
      },
      output: {
        entryFileNames: "[name].js",
        chunkFileNames: "chunks/[name].js",
        assetFileNames: "[name][extname]",
      },
    },
  },
});
