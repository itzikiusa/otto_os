---
id: rooms
title: Rooms
group: Work
route: rooms
summary: Pair on a live session with chat, voice, shared screens, permissioned drawing and terminal control, then review a transcript and recap.
---

## Start a room

1. Open a running local agent or shell session and choose **Start room…** from its More menu.
2. Enter your display name. On desktop, the room opens in a separate native window, so your workspace stays available. **Open room** in the Rooms lobby focuses that window again.
3. Choose **Invite someone…**, select **View only** or **Can control when granted**, and create an invitation. It is single-use and expires after ten minutes.
4. Your teammate opens **Rooms → Join room**, pastes the invitation and requests entry. Admit them when you are ready.

A room holds the host and up to three guests. It shares one local session, including available terminal history. Ending the room disconnects guests while the underlying session keeps running.

## Talk, present and point things out

Room chat is separate from the agent conversation. Choose **Join audio**, then turn on your microphone to speak.

Use **Share screen…** to choose a source from the platform's capture picker. Available screens, windows and tabs depend on the browser and operating system. Multiple people can present after the host grants permission; each person shares one source at a time. Pin a screen to focus on it, or return to the grid. Your choice does not change anyone else's layout.

Request annotation permission to point, highlight or draw over another person's shared screen. The presenter approves access. Marks appear in Otto over that source; they do not edit the presented application.

## Give and take back control

A guest with editor access can **Request control**. The host chooses **Give control…** to let that person type into the shared terminal. Only one person controls it at a time. The host can **Take back control** whenever needed.

Terminal control allows commands with that session's permissions. It does not grant mouse or keyboard control of other applications on the host's computer.

## Capture a transcript and summary

1. Choose **Prepare a full recap…**. For speech, configure an installed local whisper.cpp executable and a multilingual model in recap settings.
2. Ask everyone to consent. After everyone agrees, select **Start capture**. The banner shows when capture is active; a new participant, disconnection or withdrawn consent pauses it.
3. Choose **Finish recap** and allow pending recognition to finish. Open the recap to review **Transcript** and **Activity & screens**: speech, chat, terminal output, presentation activity, annotations and saved screen samples.
4. In **Summary**, choose **Generate summary…**. After confirmation, Codex uses your existing ChatGPT subscription sign-in to produce a draft with an overview, decisions, actions and open questions. No API key is needed. Review the draft before using it.

The full accepted archive stays on the host and can be exported. Screen images are periodic samples, not a continuous recording. Missing or failed recognition appears as a coverage gap. Summary generation sends captured text and selected shared images to Codex and uses your subscription allowance.

## Connecting other computers

Starting a room does not expose your Mac to the internet. Remote guests need a reachable HTTPS origin serving your Otto instance, configured in **Rooms → Connection settings**. Voice and screen sharing may also need STUN/TURN relay configuration. Chat and terminal access can work even when media cannot connect.

## Related

- [Agents](#/walkthroughs/agents)
- [Phone and remote access](#/walkthroughs/phone-and-remote)
