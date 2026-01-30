/** Mock web dev server - simulates HMR updates for demo purposes */

console.log(`Web dev server starting...`);
console.log(`Bundling assets...`);
await new Promise((r) => setTimeout(r, 500));
console.log(`Ready at http://localhost:5173`);

let hmrCount = 0;

const interval = setInterval(() => {
  hmrCount++;
  const files = ["App.tsx", "Header.tsx", "Button.tsx", "styles.css", "utils.ts"];
  const file = files[Math.floor(Math.random() * files.length)];
  console.log(`[HMR] ${file} updated`);
}, 4000);

Deno.addSignalListener("SIGTERM", () => {
  console.log(`\nShutting down dev server...`);
  clearInterval(interval);
  Deno.exit(0);
});

Deno.addSignalListener("SIGINT", () => {
  console.log(`\nDev server stopped`);
  clearInterval(interval);
  Deno.exit(0);
});
