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
