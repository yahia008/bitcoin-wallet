// A tiny static file server for the test build (node e2e/serve.mjs <dir> <port>). The site
// is a static export, so this is all it needs; no extra dependency.
import { createReadStream, statSync } from "node:fs";
import { createServer } from "node:http";
import { extname, join, normalize } from "node:path";

const [dir, port] = process.argv.slice(2);
const types = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".css": "text/css",
  ".wasm": "application/wasm",
  ".json": "application/json",
  ".txt": "text/plain",
  ".ico": "image/x-icon",
  ".svg": "image/svg+xml",
};

createServer((req, res) => {
  const path = normalize(decodeURIComponent(new URL(req.url, "http://x").pathname));
  let file = join(dir, path);
  try {
    if (statSync(file).isDirectory()) file = join(file, "index.html");
    statSync(file);
  } catch {
    res.writeHead(404).end("not found");
    return;
  }
  res.writeHead(200, { "content-type": types[extname(file)] ?? "application/octet-stream" });
  createReadStream(file).pipe(res);
}).listen(Number(port), "127.0.0.1");
