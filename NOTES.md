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

# Notes

## 10/5/2025

I'm in the middle of the podman refactor - pulling out my home grown system and what not in order to remove the "static binary" use case as I bet it will not hit much adoption - in fact - I should go after self hosted Vercel / NextJS as a first target since that's what LLMs are shilling these days. 

Let's make that use case REALLY good. So What? 

Here are my use cases: 

1. I should be able to add a new app
2. I can go ahead and connect it to an existing repository in github 
3. It will pull and build that repository
4. I will get a URL subdomain that I can visit for that service 
5. I can go on vacation and my service runs and is secure 
6. I can scale / migrate without worry. Because of litestream.

Where are we today? 

- [X] Create an app
- [ ] Connect to github (webhook support - Phase 2)
- [X] Manually set a remote
- [X] Build app based on config
- [X] Start app if it is built
- [X] See the app at a URL
    - [X] Start system services (Caddy)
    - [X] Reload config when something changes (via Caddy API)
- [X] Stop the app
- [X] Delete the app
- [X] Restart the app (stop + start)

Developer happiness
- [ ] How do I know what env I'm pointing to? Perhaps in the logs
- [ ] Make the app auto understood based on location in the cli (context)
- [ ] Put cursor IN the bindrop and give it access to the primitives. as an app woah. allow it to run with privalages.

## Phase 1 Status: COMPLETE ✓

All core functionality implemented:
- ✅ Podman integration
- ✅ Caddy reverse proxy with dynamic config
- ✅ App lifecycle (create, build, start, stop, delete)
- ✅ Git remote management
- ✅ Environment variables
- ✅ Container logs streaming
- ✅ Subdomain routing (local dev: *.localhost:9090, prod: *.s.danbruder.com)

**Testing blocked by:** Network restrictions preventing Caddy image pull
**Documentation:** See ROUTING_STATUS.md and DEPLOYMENT_GUIDE.md

NEXT: Phase 2 - Web UI & GitHub Webhooks
