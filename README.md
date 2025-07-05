# V2

For V2, I'd like to do a number of things: 
- Use production proxy like Caddy AND use podman or docker. Basically don't do stuff on my own lol. 
- They would be abstracted into orchestrator and proxy layers. I'm so concerned with abstracting out. I should not be. 
- I need to use Sqlite thing of stuff. 
- Build from github webhooks. 

What would need to change here? 
- Need a build step from webhook of github
- Start needs to run podman instead of directly, so does docker

When do I need caching and when do I not? For start, stop, restart, status, etc it is probably better to go right to podman to get the latest version. For things like reporting dashboard and history, I could listen to events.

Flow: 
1. New Code
2. New Docker image
3. New deployment of podman container 
4. New proxy config 


Some commands are driven and some reactive
1. Create, delete, - needs valid inputs (existence). Push to the database (I own this stuff)
2. deploy (build) 
3. Start, stop, restart - act on something that exists (run state). Replicate state to the database (I don't own this stuff)
4. Logs, status - reactive to something that exists (telemetry)

For phase 2, I want to use podman to start a container. I want to make sure an sqlite database is mounted and part of the litestream config, and then I want to update the caddyfile for the reverse proxy.

Say an image exists, when I say start, it would check to see if it is currently running (need podman here - do for now?) use app name for container name.
if it is not running, then start the container. I can subscribe to state changes to reflect from podman.

Can I complete item 2?  Can I complete item 1? 

I want to store the run state so that I can report on it in the UI. For checking if it is _really_ running, I call out to podman.

Which things does podman own the data for? start, stop, restart. In effect, I'm asking another system to do something and it will tell me what it has done.

Let's focus on item 3 - start stop restart
