/** Mock database - simulates query logging for demo purposes */

console.log(`Database starting...`);
console.log(`Loading data from disk...`);
await new Promise((r) => setTimeout(r, 300));
console.log(`Database ready on port 5432`);

let queryCount = 0;

const interval = setInterval(() => {
  queryCount++;
  const queries = ["SELECT * FROM users", "INSERT INTO orders", "UPDATE products", "DELETE FROM sessions"];
  const query = queries[Math.floor(Math.random() * queries.length)];
  const ms = Math.floor(Math.random() * 10) + 1;
  console.log(`Query: ${query} (${ms}ms)`);
}, 5000);

Deno.addSignalListener("SIGTERM", () => {
  console.log(`\nDatabase shutting down...`);
  console.log(`Processed ${queryCount} queries`);
  clearInterval(interval);
  Deno.exit(0);
});

Deno.addSignalListener("SIGINT", () => {
  console.log(`\nDatabase stopped`);
  clearInterval(interval);
  Deno.exit(0);
});
