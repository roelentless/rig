/** Mock background worker - simulates job processing for demo purposes */

console.log(`Worker starting...`);
console.log(`Connected to queue`);
console.log(`Worker ready, waiting for jobs`);

let jobCount = 0;

const interval = setInterval(() => {
  jobCount++;
  const jobs = ["email:send", "image:resize", "report:generate", "cache:invalidate", "notification:push"];
  const job = jobs[Math.floor(Math.random() * jobs.length)];
  const duration = Math.floor(Math.random() * 200) + 50;
  console.log(`Processing ${job} (${duration}ms)`);
}, 3000);

Deno.addSignalListener("SIGTERM", () => {
  console.log(`\nReceived SIGTERM, finishing current job...`);
  console.log(`Completed ${jobCount} jobs`);
  clearInterval(interval);
  Deno.exit(0);
});

Deno.addSignalListener("SIGINT", () => {
  console.log(`\nReceived SIGINT, aborting...`);
  clearInterval(interval);
  Deno.exit(0);
});
