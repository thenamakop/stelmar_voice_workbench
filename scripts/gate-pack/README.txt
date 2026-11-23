STELMAR GATE PACK
=================

What this is
------------
A folder that tests the STELMAR Voice Workbench recorder and speech engines on THIS laptop, with no
network and nothing to install. It runs a list of checks one after another, asks you for the few
things only a person can do (move a cable, switch Wi-Fi off, type RESTART), and at the end makes one
zip file of results. Someone then reads that zip on the development computer.

You do not need to understand the checks. You need to follow the screen.

What to bring for each laptop
-----------------------------
 1. This folder on the USB drive (about 5 GB).
 2. The laptop's charger. The laptop must stay on mains power the whole time.
 3. The USB audio interface, with one short audio cable from an OUTPUT socket of the interface to an
    INPUT socket of the same interface (the "loop-back" cable). Plug the interface into the laptop.
 4. An external SSD, if there is one (recommended, not required). While the recorder checks record, the
    app copies each test recording to a second place (its "mirror"), and one check compares the copy
    with the original. On an SSD that proves the copy works on a separate drive. Without an SSD, press
    Enter when the pack asks for the drive letter: every check still runs and passes or fails on its
    own; the copy then stays on the laptop's own drive and the results say so, and that one result
    (the copy on a separate drive) stays open until a run with an SSD. If you type a letter and the
    drive is not there, the pack says so and asks again. If the SSD is unplugged later, the recorder
    checks show "waiting" with that reason (never FAIL): plug it in and run Start-Gates.cmd again.
 5. Only if this pack has the microphone test (Start-Gates.cmd -Status lists it): a quiet room for
    it (about 2.5 hours, near the end). Do not talk during it.

Before you start (5 minutes)
----------------------------
 1. Copy the whole folder from the USB drive to the laptop's OWN internal drive (for example
    C:\STELMAR-Gate\gate-pack). Do not run it from the USB drive: the models and the recording must
    be on the same disk, and a USB drive is slower and gives wrong answers.
 2. Close these programs if they are open (they disturb the measurements): LatencyMon, RTSS (RivaTuner),
    MSI Afterburner, Steam, Nahimic and any other "audio booster", game overlay or screen recorder.
    Also close animated-wallpaper programs (Wallpaper Engine, Lively Wallpaper): on 18 Nov 2026
    Wallpaper Engine kept 45 to 90 percent of a laptop's integrated graphics busy with nothing else
    running, which makes every speed number wrong. Use a plain still wallpaper while the pack runs.
    ALSO switch off their "start with Windows" setting BEFORE the restart (Steam: Settings, Interface,
    "Run Steam when my computer starts"; Task Manager, Startup apps, for the others). Otherwise they
    start again when you log in after the restart. The pack lists the programs it finds when you start
    it, asks you to close them and press Enter, and checks again (twice at most, then it carries on).
    After the restart nobody is at the keyboard: it waits up to 10 minutes for them to close and then
    carries on, so a program that came back costs time but never loses the one-hour window. Every
    result says which of these programs were running.
 3. Plug in the charger. Wi-Fi stays ON until the pack asks you to switch it off.
 4. Do not run anything else on the laptop while the pack runs. Let the screen stay on.

How to start
------------
 Right-click  Start-Gates.cmd  and choose "Run as administrator", then answer yes to the Windows
 question. (Administrator rights are needed to clear the file cache for the cold-start checks and to
 register the "carry on after the restart" task. The pack changes no security settings.)

 The first time it asks: "LOW laptop or MID laptop? Type L or M". Use L for the weakest laptop and M
 for the other one. Type it once. After that it remembers.

 If Windows or your antivirus blocks a file, the pack says which one and stops. Do NOT turn protection
 off. Tell the pack owner which file and what Windows said.

What it does, in order
----------------------
 The pack works through a list. The most important checks come first. The screen shows what it is
 doing and about how long is left. The list is saved after every check, so you can close the window
 or the laptop can restart and it carries on where it stopped (run Start-Gates.cmd again as
 administrator and it continues).

 Day 1 (about 6 hours per laptop):
   - The laptop describes itself and checks its programs (15 minutes).
   - The quick system check and the speed tables on the graphics chip (1 to 2 hours, no cables needed).
   - The restart check. The pack says "A restart is needed" and waits. Type RESTART and press Enter.
     The laptop restarts, you log in, and the pack carries on by itself within a minute. Do not
     start anything yourself after logging in. Let the pack's window open on its own.
   - The recorder checks on the audio interface: 30 minutes under heavy load straight after the
     restart, then a 2-hour recording, then panic and kill tests (about 4 hours in all).
   - The network test: the pack asks you to switch Wi-Fi OFF (and unplug any network cable), runs a
     25-minute test, then asks you to switch it back ON. Do not forget to switch it back on.

 Day 2 (about 11 to 13 hours per laptop; see "Two laptops" below):
   - A speed check with and without language detection (English and Hindi, for the daytime and the night
     pass, on the settings this laptop's own quick check chose). It decodes the same few minutes of mixed
     English and Hindi twice, once with the language fixed and once with detection on every sentence, and
     compares the two: about 35 minutes in a full run, about 10 minutes in a short test run. It needs no
     cable and gives numbers only, no pass or fail.
   - More recorder checks (cold-cache hashing, background decoding), the speed tables on the
     processor, a full system check, and, if this pack has it, the 2.5-hour microphone test in a quiet room.
   - The engine kill tests (about 80 minutes in a full run, no cable needed): the speech engine is stopped
     by force at random points while it works, and the pack checks that the app restarts it and the
     finished text is the same as an uninterrupted run. This checks recovery only.

Your own settings are safe
--------------------------
 The checks that start the app give it a settings folder of its own inside the pack's "work" folder. They
 never read or change the real STELMAR Voice Workbench settings on this laptop (the recordings folder, the
 saved system check, the model list). If you set the environment variable WB_GATE_DATA_DIR to the real
 settings folder, the pack refuses to start and says so. The two checks that must run the installed
 (release) app, the network test, copy the three settings files aside first and put them back afterwards.

 You can stop at any time. Anything not finished is marked "waiting" in the results.

Two laptops, one interface
--------------------------
 There is only one audio interface (and, if you have it, one SSD) for both laptops. The pack tells you when it needs the
 interface ("Plug the audio interface in"). When one laptop's recorder checks are done, move the
 interface, its cable and the SSD (if any) to the other laptop. Each move takes about 10 minutes: the pack tests
 the cable itself before it records anything and says so if the cable is wrong.

 Gates that need no cable (speed tables, system checks, the network test) can run on one laptop while
 the interface is on the other.

When the pack asks you something
--------------------------------
 - "A restart is needed": type RESTART and press Enter. Nothing else restarts the laptop.
 - "Switch the network OFF": switch Wi-Fi off, unplug the network cable, press Enter. Type SKIP to do
   this part later.
 - "Plug in the audio interface": plug it in, then press Enter.
 - "Close LatencyMon / RTSS...": close the named programs, then run Start-Gates.cmd again.
 - Anything in red: read it aloud to the pack owner. It says what to do.

If something goes wrong
-----------------------
 - The window closed or the laptop restarted by itself: run Start-Gates.cmd as administrator again.
 - You want to see what is done: right-click Start-Gates.cmd, Run as administrator is not needed for
   this:  Start-Gates.cmd -Status
 - Run one check again:  Start-Gates.cmd -Only soak     (use the name shown on the screen)
 - Repeat the checks that failed:  Start-Gates.cmd -Retry
 - The pack changed the volume, the microphone mute or the power plan and you want it back at once:
   Start-Gates.cmd -Cleanup     (it also removes the "carry on after restart" task)
 - Check that no file of the pack was damaged when it was copied:  Start-Gates.cmd -Verify

When it is finished
-------------------
 At the end the screen says "end of this run" and lists every check: PASS, FAIL, INFO (numbers for
 people to read), INVALID (the run was disturbed; run it again) or "waiting" (needs something it did
 not have; the reason is shown). It also lists anything it left changed on the laptop. Then:
   1. Run  Start-Gates.cmd -Cleanup  once, so volume, mute and the restart task are put back.
   2. Switch Wi-Fi back ON if it is off.
   3. Copy the file  gate-results-<laptop name>-<date and time>.zip  (next to this README) to the USB
      drive and give it to the pack owner. It holds logs and numbers only: no audio recordings of speech,
      no models. The zip may be copied more than once during the two days; the newest copy of the same
      name has everything. A run with a different work folder (or label) writes its own zip with another
      name and never replaces the first: copy every gate-results-*.zip.

What stays on the laptop
------------------------
 The folder  work\  inside the pack holds the recordings of the test signal (and, if the microphone
 test was run, its 10-minute microphone sample). They are not in the zip. Delete the whole pack folder
 when the pack owner says the results are in. The SSD (if used) holds only copies of the test recordings.

This pack never connects to the network, sends nothing anywhere, and installs nothing. It needs no
development tools: everything it runs is in this folder.

Test audio and licences
-----------------------
 The speech clips in the data folder are taken from FLEURS (Conneau et al., "FLEURS: Few-shot Learning
 Evaluation of Universal Representations of Speech", 2022; published by Google), which is licensed under
 Creative Commons Attribution 4.0 International (CC BY 4.0): https://creativecommons.org/licenses/by/4.0/
 The clips are cut to short excerpts and some are joined or resampled for the checks. No change makes them
 endorsed by the authors or by Google. The speech models in the models folder keep their own licences,
 listed in models\manifest.json.
