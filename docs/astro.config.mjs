import { defineConfig } from "astro/config"
import mdx from "@astrojs/mdx"
import { mdbookCompat } from "./src/lib/remark-mdbook.mjs"

const copyButtonTransformer = {
  name: "moltis-copy-button",
  pre(node) {
    node.properties["data-code"] = this.source
  },
}

export default defineConfig({
  site: "https://docs.moltis.org",
  integrations: [mdx()],
  build: {
    format: "file",
  },
  markdown: {
    remarkPlugins: [mdbookCompat],
    shikiConfig: {
      themes: {
        light: "min-light",
        dark: "github-dark",
      },
      transformers: [copyButtonTransformer],
    },
  },
})
