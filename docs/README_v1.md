# bindrop

the supervisor module is a nice hub of activity. Right now we're using sqlite as a "proxy" for interacting with the system, when I think it should actually all go through the supervisor. We're also using sqlite state and in process state for 'running' status of each app. 

There's some cruft around health checks, or rather extra features that I haven't used that could be pulled out into its own area. 

The architecture is not very clean or uniform, with different layers or concerns at different levels. For example, mixing updating a struct and saving it to the database with making process changes all within the supervisor. 

In the proxy, we embed an admin dashboard and API right in the proxy. I'd rather have it be an app that we run? Or maybe it is a special app still but the logic is not IN the proxy? Don't want to take proxy resources - although we wouldn't be because it is running in the same tokio threadpool. 

There are hardening things that could be addressed - like we have the state distributed between what's actually running, what the supervisor thinks, and what is stored in sqlite. What is actually running is the source of truth. It gets polled by the supervisor AND edited when a change comes in. What if that was all one space to update state and get state? 

An in-memory store of the state with updates would be interesting.

There's a lot that could be added to the interface - exposing functionality in the UI. Also, would like to be able to run from local CLI or on the server. 

I also haven't done anything to monitor the server. like keep track of the storage space, back up the sqlite databases etc. And haven't figured out the litestream integration. Maybe it is as simple as a COPY every day.

AND would love to be able to migrate to a new server with a few commands that would be amazing. Also - where is the binary stored? S3 was interesting. System apps would help with this.

I'm drawn to making the supervisor the hub, then making the access and interaction unified.

The supervisor is accessed from the local CLI, but we need to target the correct process (the one that is running). Therefore, need to connect to it and send it commands. This can be done over CLI locally OR remotely.

Right now it only is concerned with starting, stopping, and restarting. Why should it be concerned with creating apps? Why does that need to go through the supervisor? It doesn't. But it does need to be exposed over the network b/c right now we're using the database as a shared comms bus.

The proxy needs to know about the supervisor so it can check app state on request. 

The admin API needs to know about the supervisor so it can send updates when the user interacts with the api. I do not want to add to the current API interface.
 
Also want to make the API authenticated - and a flow to do that. i.e. generate a secret with a new command line, store it in the database, then load it at run time. But that can come later. 

Need to clean up the "directory stuff" - right now it is a bit of a mess

New supervisor. What does it need to do? For each app, it should only be working on one action at a time. Start, Stop, Restart, etc. An actor for each application would be best. Also, when it receives a command, it should clear other commands as needed. For example, if I want to stop an app, I don't care about a health check. What if all stuff went through an actor like that? Because we don't want to constrain things on one single slow app action.
