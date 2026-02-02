/** Mock API server: simulates HTTP request handling for demo purposes */

const PORT = Deno.env.get("PORT") || "3000";

console.log(`API server starting on port ${PORT}...`);
console.log(`Connected to database`);
console.log(`API ready at http://localhost:${PORT}`);

let requestCount = 0;

const interval = setInterval(() => {
  requestCount++;
  const endpoints = ["GET /users", "POST /orders", "GET /products", "PUT /cart", "DELETE /session"];
  const endpoint = endpoints[Math.floor(Math.random() * endpoints.length)];
  const ms = Math.floor(Math.random() * 50) + 5;
  console.log(`${endpoint} - ${ms}ms`);
}, 2000);

Deno.addSignalListener("SIGTERM", () => {
  console.log(`\nReceived SIGTERM, shutting down...`);
  console.log(`Processed ${requestCount} requests`);
  clearInterval(interval);
  Deno.exit(0);
});

Deno.addSignalListener("SIGINT", () => {
  console.log(`\nReceived SIGINT, shutting down...`);
  clearInterval(interval);
  Deno.exit(0);
});
