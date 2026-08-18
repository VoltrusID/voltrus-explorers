const fs = require("fs");
const path = process.argv[2] || "/tmp/voltrus-hmi-export-test.html";
const out = "/tmp/voltrus-hmi-export-test.js";

const html = fs.readFileSync(path, "utf8");
const m = html.match(/<script>([\s\S]*)<\/script>/);
if (!m) {
  console.error("NO SCRIPT BLOCK FOUND");
  process.exit(1);
}
fs.writeFileSync(out, m[1]);
console.log("script bytes:", m[1].length);
