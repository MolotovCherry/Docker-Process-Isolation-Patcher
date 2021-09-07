# Docker Process Isolation Patcher
This program automatically patches dockerd service to run in process isolation mode.

## Why?
Process isolation is better, period. It's faster, easier, and just... better. While docker now allows you to run this flag in Windows clients, the way it currently is, you have to always add manually add the `--isolation=process` flag every time. What a pain!

## What does this do?
When docker is in Windows container mode, it uses a service process `dockerd`, which does not run in process isolation mode, which is why we have to always add that flag. However, `dockerd` does have that flag in it. What this program does is, it watches for the `dockerd` service, and any time it starts up / is created, it stops the `dockerd` service, patches it to add that flag, then starts it up again in process isolation mode.

## How to run?
We have a couple different program arguments to manage it.

Note: This program must be run in administrator mode.

1. Move your program to a final location.
2. Install the service by using the flags below.
3. Start the service.

| Flag              | Description                                                        |
|-------------------|--------------------------------------------------------------------|
| install-service   | installs the patcher service                                       |
| uninstall-service | uninstalls the patcher service                                     |
| start-service     | starts the patcher service                                         |
| run-service       | windows services runs this flag internally. don't call it manually |
| stop-service      | stop the patcher service                                           |

Of course, you can also manually start/stop/restart the service in the Windows services manager.

## Where are the binaries?
Check the release section for a binary!

## Reporting bugs
The app automatically logs to `app.log` in the same directory as the exe. If you encounter a crash, please make an issue, detail how to reproduce the crash, and post your logfile.
