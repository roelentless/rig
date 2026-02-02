/** Mock mail server - simulates SMTP for demo purposes */

console.log(`Mail server starting...`);
console.log(`SMTP ready on port 1025`);
console.log(`Web UI ready at http://localhost:8025`);

let mailCount = 0;

const interval = setInterval(() => {
  mailCount++;
  const types = ["welcome", "password-reset", "notification", "invoice", "newsletter"];
  const type = types[Math.floor(Math.random() * types.length)];
  console.log(`Received: ${type} email (#${mailCount})`);
}, 6000);

Deno.addSignalListener("SIGTERM", () => {
  console.log(`\nMail server shutting down...`);
  console.log(`Processed ${mailCount} emails`);
  clearInterval(interval);
  Deno.exit(0);
});

Deno.addSignalListener("SIGINT", () => {
  console.log(`\nMail server stopped`);
  clearInterval(interval);
  Deno.exit(0);
});
