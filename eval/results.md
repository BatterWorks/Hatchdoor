
## NomicEmbedTextV15 — chunk 300/40 · ctx on · dim native

- Run timestamp: 2026-07-25T20:35:01Z
- Build duration: 951.9 s
- Build window: 2026-07-25T20:19:01Z → 2026-07-25T20:34:53Z
- Build peak RSS: 958.0 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.814 |
| Recall@5 (all) | 0.749 |
| Recall@10 (any) | 0.856 |
| Recall@10 (all) | 0.809 |
| MRR | 0.696 |
| FP-rate@5 | 0.253 |
| Correct-heading | 0.875 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.667 | 0.741 | 0.522 | n/a |
| heading | 24 | 0.875 | 0.958 | 0.741 | 0.875 |
| code-fragment | 14 | 0.857 | 0.857 | 0.689 | n/a |
| multi-note | 18 | 0.833 | 0.833 | 0.752 | n/a |
| exploratory | 17 | 0.706 | 0.765 | 0.530 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 1.000 | n/a |
| staleness | 7 | 0.429 | 0.429 | 0.429 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.765 | 0.812 | 0.636 | 0.895 |
| realistic | 33 | 0.939 | 0.970 | 0.848 | 0.800 |
| diagnostic | 7 | 0.429 | 0.429 | 0.429 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | 5 | no |
| C2 | which box handles name resolution at my parents' place | 1 | no |
| C3 | where does the long-running autonomous agent live | — | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | — | no |
| C5 | where are the offsite copies of my git repositories kept | 10 | no |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | — | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | 2 | no |
| C10 | the beans with the citrus and floral notes | 2 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 2 | no |
| C12 | which of my data would actually hurt to lose | 3 | no |
| C13 | what should be switched off rather than kept running after I am gone | — | no |
| C14 | something to get my dad | 2 | yes |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | 2 | — |
| H5 | how are the media drives laid out | 1 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 3 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | 10 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 4 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 1 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 5 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 1 | — |
| F7 | 21116 udp forward | 1 | no |
| F8 | 192.168.31.230 | — | no |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 4 | no |
| F10 | vm.swappiness=10 | 5 | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | 1 | no |
| F13 | trivy-fleet-audit.timer | 1 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 1 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | no |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 1 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 1 | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | — | no |
| M7 | getting the car's charge level onto my dashboards | 1 | no |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | no |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | yes |
| M11 | preparing for the maternity nurse visits | 5 | yes |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | — | no |
| M14 | where do I keep track of which bags I have already bought | — | no |
| X1 | what should we do this weekend | — | yes |
| X2 | something to put on tonight | — | no |
| X3 | I want to buy something nice for the flat | 3 | yes |
| X4 | what am I meant to be reading | 1 | no |
| X5 | an idea I could actually sit down and build | 10 | no |
| X6 | keeping the machines patched and scanned for holes | — | yes |
| X7 | how do I handle people I find hard to deal with | 2 | yes |
| X8 | the general routine for looking after the indoor plants | 2 | yes |
| X9 | how am I going about picking up the language | 1 | no |
| X10 | what should I print next | 4 | no |
| X11 | ways to give an assistant a memory that persists | 1 | no |
| X12 | how would I find out a disk is dying before it takes something with it | — | no |
| X13 | the rules for keeping these notes tidy | 1 | no |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | no |
| X16 | how are we going to handle solids when the time comes | 1 | no |
| X17 | cheaper ways to rent compute | 3 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 1 | yes |
| H12 | what runs overnight, hour by hour | 1 | yes |
| H13 | what happens if the key server is unreachable when a machine reboots | 1 | no |
| H14 | which of the two feeds should I actually point the indexer at | 1 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 1 | no |
| H18 | which guest was actually filling up the backup store | — | no |
| H19 | how do I get an off-site agent talking again after its key drifts | 2 | no |
| H20 | what has changed on the little N100 machine lately | 9 | no |
| H21 | which packages did I deliberately tell the bot to leave alone | 2 | no |
| N1 | Kokuyo Campus notebook buying notes | 1 | no |
| C16 | which of the plants gets watered on a fixed weekly schedule | 1 | yes |
| C17 | the one I should test with a finger instead of watering to a routine | — | no |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | 3 | yes |
| C21 | how many kicks should I feel in two hours before ringing someone | 8 | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | — | no |
| C24 | who do I call when a pipe leaks | 1 | no |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | — | yes |
| S2 | what does network-wide DNS filtering run on | — | no |
| S3 | how do I keep homelab secrets encrypted at rest | 1 | yes |
| S4 | how are container image updates automated | — | yes |
| S5 | how is the browser terminal exposed | — | no |
| S6 | the script that sets up my shell on a freshly built machine | 1 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 2 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 1 | — |
| U2 | I'm looking for a new smell for the house | 3 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 1 | yes |
| U5 | I am travelling by plane with the baby | 1 | no |
| U6 | How can I reflect on things with my family? | 1 | yes |

## NomicEmbedTextV15 — chunk 300/40 · ctx off · dim native

- Run timestamp: 2026-07-25T20:48:53Z
- Build duration: 821.4 s
- Build window: 2026-07-25T20:35:03Z → 2026-07-25T20:48:44Z
- Build peak RSS: 966.0 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.814 |
| Recall@5 (all) | 0.763 |
| Recall@10 (any) | 0.881 |
| Recall@10 (all) | 0.839 |
| MRR | 0.660 |
| FP-rate@5 | 0.253 |
| Correct-heading | 0.583 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.815 | 0.852 | 0.542 | n/a |
| heading | 24 | 0.708 | 0.833 | 0.581 | 0.583 |
| code-fragment | 14 | 0.786 | 0.857 | 0.741 | n/a |
| multi-note | 18 | 0.778 | 0.889 | 0.675 | n/a |
| exploratory | 17 | 0.824 | 0.882 | 0.541 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 0.972 | n/a |
| staleness | 7 | 0.429 | 0.571 | 0.378 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.788 | 0.859 | 0.593 | 0.632 |
| realistic | 33 | 0.879 | 0.939 | 0.830 | 0.400 |
| diagnostic | 7 | 0.429 | 0.571 | 0.378 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | 3 | no |
| C2 | which box handles name resolution at my parents' place | 2 | no |
| C3 | where does the long-running autonomous agent live | — | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 9 | no |
| C5 | where are the offsite copies of my git repositories kept | — | no |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | — | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | 2 | no |
| C10 | the beans with the citrus and floral notes | 2 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 2 | no |
| C12 | which of my data would actually hurt to lose | 2 | no |
| C13 | what should be switched off rather than kept running after I am gone | 4 | no |
| C14 | something to get my dad | 3 | yes |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | — | — |
| H5 | how are the media drives laid out | 1 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | — | — |
| H7 | which clients are currently connected to the VPN | 2 | — |
| H8 | what to check first when name resolution breaks | 8 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 3 | — |
| H10 | what were the main design decisions behind the deployment pipeline | — | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 1 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 1 | — |
| F7 | 21116 udp forward | 1 | no |
| F8 | 192.168.31.230 | — | no |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 8 | no |
| F10 | vm.swappiness=10 | 4 | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | 1 | no |
| F13 | trivy-fleet-audit.timer | 1 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 1 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | no |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 1 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 5 | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | — | no |
| M7 | getting the car's charge level onto my dashboards | 1 | no |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | no |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | yes |
| M11 | preparing for the maternity nurse visits | 6 | yes |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | — | no |
| M14 | where do I keep track of which bags I have already bought | 8 | no |
| X1 | what should we do this weekend | 2 | no |
| X2 | something to put on tonight | — | no |
| X3 | I want to buy something nice for the flat | 1 | yes |
| X4 | what am I meant to be reading | 1 | no |
| X5 | an idea I could actually sit down and build | 5 | no |
| X6 | keeping the machines patched and scanned for holes | 3 | yes |
| X7 | how do I handle people I find hard to deal with | 3 | yes |
| X8 | the general routine for looking after the indoor plants | 3 | yes |
| X9 | how am I going about picking up the language | 1 | no |
| X10 | what should I print next | — | no |
| X11 | ways to give an assistant a memory that persists | 2 | no |
| X12 | how would I find out a disk is dying before it takes something with it | 6 | no |
| X13 | the rules for keeping these notes tidy | 1 | no |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 2 | no |
| X16 | how are we going to handle solids when the time comes | 1 | no |
| X17 | cheaper ways to rent compute | 3 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 1 | yes |
| H12 | what runs overnight, hour by hour | 7 | yes |
| H13 | what happens if the key server is unreachable when a machine reboots | 5 | yes |
| H14 | which of the two feeds should I actually point the indexer at | 4 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 1 | no |
| H18 | which guest was actually filling up the backup store | 4 | no |
| H19 | how do I get an off-site agent talking again after its key drifts | 1 | no |
| H20 | what has changed on the little N100 machine lately | — | no |
| H21 | which packages did I deliberately tell the bot to leave alone | 7 | no |
| N1 | Kokuyo Campus notebook buying notes | 1 | no |
| C16 | which of the plants gets watered on a fixed weekly schedule | 1 | yes |
| C17 | the one I should test with a finger instead of watering to a routine | 5 | no |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | 4 | yes |
| C21 | how many kicks should I feel in two hours before ringing someone | 3 | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 3 | no |
| C24 | who do I call when a pipe leaks | 1 | no |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | 7 | yes |
| S2 | what does network-wide DNS filtering run on | — | no |
| S3 | how do I keep homelab secrets encrypted at rest | 2 | yes |
| S4 | how are container image updates automated | — | yes |
| S5 | how is the browser terminal exposed | — | no |
| S6 | the script that sets up my shell on a freshly built machine | 1 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 1 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 2 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 1 | — |
| U2 | I'm looking for a new smell for the house | 3 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 3 | no |
| U5 | I am travelling by plane with the baby | 1 | yes |
| U6 | How can I reflect on things with my family? | 1 | yes |

## NomicEmbedTextV15 — chunk 450/50 · ctx on · dim native

- Run timestamp: 2026-07-25T21:04:15Z
- Build duration: 912.6 s
- Build window: 2026-07-25T20:48:55Z → 2026-07-25T21:04:07Z
- Build peak RSS: 1065.1 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.788 |
| Recall@5 (all) | 0.742 |
| Recall@10 (any) | 0.890 |
| Recall@10 (all) | 0.839 |
| MRR | 0.702 |
| FP-rate@5 | 0.265 |
| Correct-heading | 0.917 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.667 | 0.815 | 0.520 | n/a |
| heading | 24 | 0.833 | 0.958 | 0.790 | 0.917 |
| code-fragment | 14 | 0.714 | 0.786 | 0.633 | n/a |
| multi-note | 18 | 0.778 | 0.944 | 0.715 | n/a |
| exploratory | 17 | 0.765 | 0.824 | 0.598 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 1.000 | n/a |
| staleness | 7 | 0.429 | 0.429 | 0.429 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.753 | 0.871 | 0.637 | 0.895 |
| realistic | 33 | 0.879 | 0.939 | 0.871 | 1.000 |
| diagnostic | 7 | 0.429 | 0.429 | 0.429 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | — | no |
| C2 | which box handles name resolution at my parents' place | 1 | no |
| C3 | where does the long-running autonomous agent live | — | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | — | no |
| C5 | where are the offsite copies of my git repositories kept | 9 | no |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | — | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | 2 | no |
| C10 | the beans with the citrus and floral notes | 2 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 2 | yes |
| C12 | which of my data would actually hurt to lose | 2 | no |
| C13 | what should be switched off rather than kept running after I am gone | 6 | no |
| C14 | something to get my dad | 2 | no |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | 1 | — |
| H5 | how are the media drives laid out | 1 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 2 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | 7 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 2 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 1 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 5 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 6 | — |
| F7 | 21116 udp forward | 1 | no |
| F8 | 192.168.31.230 | — | no |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 2 | no |
| F10 | vm.swappiness=10 | — | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | 1 | no |
| F13 | trivy-fleet-audit.timer | 1 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 1 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | no |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 1 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 2 | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | — | no |
| M7 | getting the car's charge level onto my dashboards | 1 | no |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | no |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 10 | yes |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | 6 | no |
| M14 | where do I keep track of which bags I have already bought | 10 | no |
| X1 | what should we do this weekend | 5 | yes |
| X2 | something to put on tonight | — | no |
| X3 | I want to buy something nice for the flat | 3 | yes |
| X4 | what am I meant to be reading | 1 | no |
| X5 | an idea I could actually sit down and build | 8 | no |
| X6 | keeping the machines patched and scanned for holes | — | yes |
| X7 | how do I handle people I find hard to deal with | 2 | yes |
| X8 | the general routine for looking after the indoor plants | 2 | yes |
| X9 | how am I going about picking up the language | 1 | no |
| X10 | what should I print next | 1 | no |
| X11 | ways to give an assistant a memory that persists | 1 | no |
| X12 | how would I find out a disk is dying before it takes something with it | — | no |
| X13 | the rules for keeping these notes tidy | 1 | no |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | no |
| X16 | how are we going to handle solids when the time comes | 1 | no |
| X17 | cheaper ways to rent compute | 2 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 1 | no |
| H12 | what runs overnight, hour by hour | 1 | yes |
| H13 | what happens if the key server is unreachable when a machine reboots | 1 | yes |
| H14 | which of the two feeds should I actually point the indexer at | 1 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 1 | no |
| H18 | which guest was actually filling up the backup store | 7 | no |
| H19 | how do I get an off-site agent talking again after its key drifts | 6 | no |
| H20 | what has changed on the little N100 machine lately | — | no |
| H21 | which packages did I deliberately tell the bot to leave alone | 2 | yes |
| N1 | Kokuyo Campus notebook buying notes | 1 | yes |
| C16 | which of the plants gets watered on a fixed weekly schedule | 2 | yes |
| C17 | the one I should test with a finger instead of watering to a routine | 6 | no |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | 4 | yes |
| C21 | how many kicks should I feel in two hours before ringing someone | 4 | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 10 | no |
| C24 | who do I call when a pipe leaks | 1 | no |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | no |
| S1 | how are the DAS shares mounted for the media stack | — | yes |
| S2 | what does network-wide DNS filtering run on | — | no |
| S3 | how do I keep homelab secrets encrypted at rest | 1 | yes |
| S4 | how are container image updates automated | — | yes |
| S5 | how is the browser terminal exposed | — | no |
| S6 | the script that sets up my shell on a freshly built machine | 1 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 1 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 1 | — |
| U2 | I'm looking for a new smell for the house | 2 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 2 | yes |
| U5 | I am travelling by plane with the baby | 1 | yes |
| U6 | How can I reflect on things with my family? | 1 | yes |

## NomicEmbedTextV15 — chunk 450/50 · ctx off · dim native

- Run timestamp: 2026-07-25T21:17:46Z
- Build duration: 802.0 s
- Build window: 2026-07-25T21:04:17Z → 2026-07-25T21:17:39Z
- Build peak RSS: 966.0 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.814 |
| Recall@5 (all) | 0.757 |
| Recall@10 (any) | 0.907 |
| Recall@10 (all) | 0.857 |
| MRR | 0.685 |
| FP-rate@5 | 0.277 |
| Correct-heading | 0.667 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.704 | 0.815 | 0.573 | n/a |
| heading | 24 | 0.750 | 0.917 | 0.610 | 0.667 |
| code-fragment | 14 | 0.786 | 0.857 | 0.740 | n/a |
| multi-note | 18 | 0.889 | 0.944 | 0.681 | n/a |
| exploratory | 17 | 0.824 | 0.941 | 0.596 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 1.000 | n/a |
| staleness | 7 | 0.429 | 0.571 | 0.357 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.788 | 0.882 | 0.619 | 0.737 |
| realistic | 33 | 0.879 | 0.970 | 0.855 | 0.400 |
| diagnostic | 7 | 0.429 | 0.571 | 0.357 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | — | no |
| C2 | which box handles name resolution at my parents' place | 1 | no |
| C3 | where does the long-running autonomous agent live | — | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 10 | no |
| C5 | where are the offsite copies of my git repositories kept | — | no |
| C6 | what runs the retro gaming console plugged into the TV | 2 | no |
| C7 | which server scans and files my paperwork | — | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | 2 | no |
| C10 | the beans with the citrus and floral notes | 2 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 1 | yes |
| C12 | which of my data would actually hurt to lose | 3 | no |
| C13 | what should be switched off rather than kept running after I am gone | 1 | no |
| C14 | something to get my dad | 2 | no |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | 10 | — |
| H5 | how are the media drives laid out | 1 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 9 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | 3 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 1 | — |
| H10 | what were the main design decisions behind the deployment pipeline | — | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 1 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 1 | — |
| F7 | 21116 udp forward | 1 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 4 | no |
| F10 | vm.swappiness=10 | 9 | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | 1 | no |
| F13 | trivy-fleet-audit.timer | 1 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 1 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | no |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 1 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 4 | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | — | no |
| M7 | getting the car's charge level onto my dashboards | 1 | no |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | no |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | yes |
| M11 | preparing for the maternity nurse visits | 6 | yes |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | 4 | no |
| M14 | where do I keep track of which bags I have already bought | 4 | no |
| X1 | what should we do this weekend | 2 | no |
| X2 | something to put on tonight | — | no |
| X3 | I want to buy something nice for the flat | 2 | yes |
| X4 | what am I meant to be reading | 1 | no |
| X5 | an idea I could actually sit down and build | 3 | no |
| X6 | keeping the machines patched and scanned for holes | 6 | yes |
| X7 | how do I handle people I find hard to deal with | 2 | yes |
| X8 | the general routine for looking after the indoor plants | 2 | yes |
| X9 | how am I going about picking up the language | 1 | no |
| X10 | what should I print next | 8 | no |
| X11 | ways to give an assistant a memory that persists | 2 | no |
| X12 | how would I find out a disk is dying before it takes something with it | 2 | no |
| X13 | the rules for keeping these notes tidy | 1 | no |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | no |
| X16 | how are we going to handle solids when the time comes | 1 | no |
| X17 | cheaper ways to rent compute | 2 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 2 | yes |
| H12 | what runs overnight, hour by hour | 7 | yes |
| H13 | what happens if the key server is unreachable when a machine reboots | 3 | yes |
| H14 | which of the two feeds should I actually point the indexer at | 4 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 1 | no |
| H18 | which guest was actually filling up the backup store | 2 | no |
| H19 | how do I get an off-site agent talking again after its key drifts | 9 | no |
| H20 | what has changed on the little N100 machine lately | — | no |
| H21 | which packages did I deliberately tell the bot to leave alone | 4 | yes |
| N1 | Kokuyo Campus notebook buying notes | 1 | no |
| C16 | which of the plants gets watered on a fixed weekly schedule | 3 | yes |
| C17 | the one I should test with a finger instead of watering to a routine | 1 | no |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | 10 | yes |
| C21 | how many kicks should I feel in two hours before ringing someone | 2 | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 9 | no |
| C24 | who do I call when a pipe leaks | 1 | no |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | 6 | yes |
| S2 | what does network-wide DNS filtering run on | — | no |
| S3 | how do I keep homelab secrets encrypted at rest | 3 | yes |
| S4 | how are container image updates automated | — | yes |
| S5 | how is the browser terminal exposed | — | no |
| S6 | the script that sets up my shell on a freshly built machine | 1 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 1 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 2 | — |
| U2 | I'm looking for a new smell for the house | 2 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 3 | no |
| U5 | I am travelling by plane with the baby | 1 | yes |
| U6 | How can I reflect on things with my family? | 1 | yes |

## NomicEmbedTextV15 — chunk 800/50 · ctx on · dim native

- Run timestamp: 2026-07-25T21:32:50Z
- Build duration: 895.2 s
- Build window: 2026-07-25T21:17:48Z → 2026-07-25T21:32:43Z
- Build peak RSS: 1261.7 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.822 |
| Recall@5 (all) | 0.771 |
| Recall@10 (any) | 0.907 |
| Recall@10 (all) | 0.855 |
| MRR | 0.738 |
| FP-rate@5 | 0.277 |
| Correct-heading | 0.750 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.704 | 0.852 | 0.593 | n/a |
| heading | 24 | 0.917 | 0.917 | 0.793 | 0.750 |
| code-fragment | 14 | 0.714 | 0.857 | 0.624 | n/a |
| multi-note | 18 | 0.778 | 0.944 | 0.746 | n/a |
| exploratory | 17 | 0.824 | 0.882 | 0.700 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 1.000 | n/a |
| staleness | 7 | 0.429 | 0.429 | 0.429 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.800 | 0.894 | 0.687 | 0.895 |
| realistic | 33 | 0.879 | 0.939 | 0.872 | 0.200 |
| diagnostic | 7 | 0.429 | 0.429 | 0.429 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | — | no |
| C2 | which box handles name resolution at my parents' place | 1 | no |
| C3 | where does the long-running autonomous agent live | — | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 10 | no |
| C5 | where are the offsite copies of my git repositories kept | 6 | no |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | — | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | 2 | no |
| C10 | the beans with the citrus and floral notes | 2 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 2 | yes |
| C12 | which of my data would actually hurt to lose | 1 | no |
| C13 | what should be switched off rather than kept running after I am gone | 2 | no |
| C14 | something to get my dad | 2 | no |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | 1 | — |
| H5 | how are the media drives laid out | 1 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 1 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | — | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 1 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 1 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 4 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 7 | — |
| F7 | 21116 udp forward | 1 | no |
| F8 | 192.168.31.230 | — | no |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 4 | no |
| F10 | vm.swappiness=10 | 10 | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | 1 | no |
| F13 | trivy-fleet-audit.timer | 1 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 1 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | yes |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 1 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 1 | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | — | no |
| M7 | getting the car's charge level onto my dashboards | 1 | yes |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | no |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 6 | yes |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | 7 | no |
| M14 | where do I keep track of which bags I have already bought | 8 | no |
| X1 | what should we do this weekend | 2 | yes |
| X2 | something to put on tonight | — | no |
| X3 | I want to buy something nice for the flat | 2 | yes |
| X4 | what am I meant to be reading | 1 | no |
| X5 | an idea I could actually sit down and build | 7 | no |
| X6 | keeping the machines patched and scanned for holes | — | yes |
| X7 | how do I handle people I find hard to deal with | 2 | yes |
| X8 | the general routine for looking after the indoor plants | 1 | yes |
| X9 | how am I going about picking up the language | 1 | no |
| X10 | what should I print next | 1 | no |
| X11 | ways to give an assistant a memory that persists | 1 | no |
| X12 | how would I find out a disk is dying before it takes something with it | 4 | no |
| X13 | the rules for keeping these notes tidy | 1 | no |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | no |
| X16 | how are we going to handle solids when the time comes | 1 | no |
| X17 | cheaper ways to rent compute | 1 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 2 | no |
| H12 | what runs overnight, hour by hour | 1 | no |
| H13 | what happens if the key server is unreachable when a machine reboots | 2 | no |
| H14 | which of the two feeds should I actually point the indexer at | 2 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | yes |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 1 | no |
| H18 | which guest was actually filling up the backup store | 3 | no |
| H19 | how do I get an off-site agent talking again after its key drifts | 5 | no |
| H20 | what has changed on the little N100 machine lately | — | no |
| H21 | which packages did I deliberately tell the bot to leave alone | 1 | no |
| N1 | Kokuyo Campus notebook buying notes | 1 | yes |
| C16 | which of the plants gets watered on a fixed weekly schedule | 2 | yes |
| C17 | the one I should test with a finger instead of watering to a routine | 9 | no |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | 1 | yes |
| C21 | how many kicks should I feel in two hours before ringing someone | 2 | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 7 | no |
| C24 | who do I call when a pipe leaks | 1 | no |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | — | yes |
| S2 | what does network-wide DNS filtering run on | — | no |
| S3 | how do I keep homelab secrets encrypted at rest | 1 | yes |
| S4 | how are container image updates automated | — | yes |
| S5 | how is the browser terminal exposed | — | no |
| S6 | the script that sets up my shell on a freshly built machine | 1 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 1 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 1 | — |
| U2 | I'm looking for a new smell for the house | 2 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 2 | yes |
| U5 | I am travelling by plane with the baby | 1 | yes |
| U6 | How can I reflect on things with my family? | 1 | yes |

## NomicEmbedTextV15 — chunk 800/50 · ctx off · dim native

- Run timestamp: 2026-07-25T21:47:00Z
- Build duration: 840.2 s
- Build window: 2026-07-25T21:32:52Z → 2026-07-25T21:46:52Z
- Build peak RSS: 1255.8 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.839 |
| Recall@5 (all) | 0.777 |
| Recall@10 (any) | 0.915 |
| Recall@10 (all) | 0.864 |
| MRR | 0.708 |
| FP-rate@5 | 0.289 |
| Correct-heading | 0.667 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.741 | 0.815 | 0.628 | n/a |
| heading | 24 | 0.792 | 0.917 | 0.649 | 0.667 |
| code-fragment | 14 | 0.786 | 0.857 | 0.617 | n/a |
| multi-note | 18 | 0.889 | 1.000 | 0.734 | n/a |
| exploratory | 17 | 0.882 | 0.941 | 0.693 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 0.963 | n/a |
| staleness | 7 | 0.429 | 0.571 | 0.378 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.812 | 0.906 | 0.659 | 0.842 |
| realistic | 33 | 0.909 | 0.939 | 0.833 | 0.000 |
| diagnostic | 7 | 0.429 | 0.571 | 0.378 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | 6 | no |
| C2 | which box handles name resolution at my parents' place | 1 | no |
| C3 | where does the long-running autonomous agent live | — | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 10 | no |
| C5 | where are the offsite copies of my git repositories kept | — | no |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | — | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | 2 | no |
| C10 | the beans with the citrus and floral notes | 2 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 1 | yes |
| C12 | which of my data would actually hurt to lose | 1 | no |
| C13 | what should be switched off rather than kept running after I am gone | 1 | no |
| C14 | something to get my dad | 2 | no |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | — | — |
| H5 | how are the media drives laid out | 1 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 7 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | 8 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 2 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 4 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 1 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 9 | — |
| F7 | 21116 udp forward | 2 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 5 | no |
| F10 | vm.swappiness=10 | 3 | — |
| F11 | telegram-notify@ template unit OnFailure | 2 | no |
| F12 | mnt-tmvol.mount | 1 | no |
| F13 | trivy-fleet-audit.timer | 1 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 1 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | yes |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 1 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 2 | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | 8 | no |
| M7 | getting the car's charge level onto my dashboards | 1 | no |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | no |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 5 | yes |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | 7 | no |
| M14 | where do I keep track of which bags I have already bought | 4 | no |
| X1 | what should we do this weekend | 1 | no |
| X2 | something to put on tonight | — | no |
| X3 | I want to buy something nice for the flat | 2 | yes |
| X4 | what am I meant to be reading | 1 | no |
| X5 | an idea I could actually sit down and build | 3 | no |
| X6 | keeping the machines patched and scanned for holes | 9 | yes |
| X7 | how do I handle people I find hard to deal with | 2 | yes |
| X8 | the general routine for looking after the indoor plants | 1 | yes |
| X9 | how am I going about picking up the language | 1 | no |
| X10 | what should I print next | 3 | no |
| X11 | ways to give an assistant a memory that persists | 2 | no |
| X12 | how would I find out a disk is dying before it takes something with it | 2 | no |
| X13 | the rules for keeping these notes tidy | 1 | no |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | no |
| X16 | how are we going to handle solids when the time comes | 1 | no |
| X17 | cheaper ways to rent compute | 1 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 1 | yes |
| H12 | what runs overnight, hour by hour | 3 | yes |
| H13 | what happens if the key server is unreachable when a machine reboots | 2 | no |
| H14 | which of the two feeds should I actually point the indexer at | 1 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | yes |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 1 | no |
| H18 | which guest was actually filling up the backup store | 4 | no |
| H19 | how do I get an off-site agent talking again after its key drifts | 3 | no |
| H20 | what has changed on the little N100 machine lately | — | no |
| H21 | which packages did I deliberately tell the bot to leave alone | 7 | yes |
| N1 | Kokuyo Campus notebook buying notes | 1 | yes |
| C16 | which of the plants gets watered on a fixed weekly schedule | 2 | yes |
| C17 | the one I should test with a finger instead of watering to a routine | — | no |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | 1 | yes |
| C21 | how many kicks should I feel in two hours before ringing someone | 2 | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 5 | no |
| C24 | who do I call when a pipe leaks | 1 | no |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | 7 | yes |
| S2 | what does network-wide DNS filtering run on | — | no |
| S3 | how do I keep homelab secrets encrypted at rest | 2 | yes |
| S4 | how are container image updates automated | — | yes |
| S5 | how is the browser terminal exposed | — | no |
| S6 | the script that sets up my shell on a freshly built machine | 1 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 1 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 3 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 1 | — |
| U2 | I'm looking for a new smell for the house | 2 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 2 | no |
| U5 | I am travelling by plane with the baby | 1 | yes |
| U6 | How can I reflect on things with my family? | 1 | yes |

## GTEBaseENV15 — chunk 300/40 · ctx on · dim native

- Run timestamp: 2026-07-25T22:03:04Z
- Build duration: 953.5 s
- Build window: 2026-07-25T21:47:03Z → 2026-07-25T22:02:56Z
- Build peak RSS: 793.6 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.746 |
| Recall@5 (all) | 0.681 |
| Recall@10 (any) | 0.814 |
| Recall@10 (all) | 0.751 |
| MRR | 0.649 |
| FP-rate@5 | 0.241 |
| Correct-heading | 0.750 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.519 | 0.593 | 0.425 | n/a |
| heading | 24 | 0.875 | 0.875 | 0.772 | 0.750 |
| code-fragment | 14 | 0.571 | 0.714 | 0.469 | n/a |
| multi-note | 18 | 0.889 | 0.944 | 0.664 | n/a |
| exploratory | 17 | 0.647 | 0.824 | 0.592 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 1.000 | n/a |
| staleness | 7 | 0.429 | 0.571 | 0.310 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.706 | 0.788 | 0.584 | 0.737 |
| realistic | 33 | 0.848 | 0.879 | 0.816 | 0.800 |
| diagnostic | 7 | 0.429 | 0.571 | 0.310 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | — | no |
| C2 | which box handles name resolution at my parents' place | 6 | no |
| C3 | where does the long-running autonomous agent live | — | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | — | no |
| C5 | where are the offsite copies of my git repositories kept | — | no |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | — | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | — | no |
| C10 | the beans with the citrus and floral notes | 5 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 10 | yes |
| C12 | which of my data would actually hurt to lose | — | no |
| C13 | what should be switched off rather than kept running after I am gone | — | no |
| C14 | something to get my dad | 1 | no |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | 1 | — |
| H5 | how are the media drives laid out | 1 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 1 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | 1 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 1 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 5 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 2 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 1 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 9 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 1 | — |
| F7 | 21116 udp forward | 3 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 8 | no |
| F10 | vm.swappiness=10 | — | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | — | no |
| F13 | trivy-fleet-audit.timer | 2 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 1 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | no |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 2 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 4 | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | — | no |
| M7 | getting the car's charge level onto my dashboards | 1 | no |
| M8 | the wall shades we settled on and their fan-deck codes | 2 | no |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | yes |
| M11 | preparing for the maternity nurse visits | 3 | no |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | 2 | no |
| M14 | where do I keep track of which bags I have already bought | 9 | no |
| X1 | what should we do this weekend | 1 | no |
| X2 | something to put on tonight | — | yes |
| X3 | I want to buy something nice for the flat | 6 | yes |
| X4 | what am I meant to be reading | 1 | yes |
| X5 | an idea I could actually sit down and build | — | no |
| X6 | keeping the machines patched and scanned for holes | 9 | yes |
| X7 | how do I handle people I find hard to deal with | 1 | yes |
| X8 | the general routine for looking after the indoor plants | 3 | yes |
| X9 | how am I going about picking up the language | 3 | no |
| X10 | what should I print next | — | no |
| X11 | ways to give an assistant a memory that persists | 1 | no |
| X12 | how would I find out a disk is dying before it takes something with it | 9 | no |
| X13 | the rules for keeping these notes tidy | 1 | no |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | no |
| X16 | how are we going to handle solids when the time comes | 1 | no |
| X17 | cheaper ways to rent compute | 1 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 1 | no |
| H12 | what runs overnight, hour by hour | 1 | no |
| H13 | what happens if the key server is unreachable when a machine reboots | 2 | no |
| H14 | which of the two feeds should I actually point the indexer at | 1 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 3 | no |
| H18 | which guest was actually filling up the backup store | — | no |
| H19 | how do I get an off-site agent talking again after its key drifts | 1 | no |
| H20 | what has changed on the little N100 machine lately | 2 | yes |
| H21 | which packages did I deliberately tell the bot to leave alone | — | yes |
| N1 | Kokuyo Campus notebook buying notes | 1 | no |
| C16 | which of the plants gets watered on a fixed weekly schedule | — | yes |
| C17 | the one I should test with a finger instead of watering to a routine | 2 | yes |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | no |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | — | no |
| C21 | how many kicks should I feel in two hours before ringing someone | 2 | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 2 | yes |
| C24 | who do I call when a pipe leaks | 2 | no |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | — | yes |
| S2 | what does network-wide DNS filtering run on | — | no |
| S3 | how do I keep homelab secrets encrypted at rest | 6 | yes |
| S4 | how are container image updates automated | — | yes |
| S5 | how is the browser terminal exposed | 2 | no |
| S6 | the script that sets up my shell on a freshly built machine | 2 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | — | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 1 | — |
| U2 | I'm looking for a new smell for the house | 2 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 4 | no |
| U5 | I am travelling by plane with the baby | 1 | no |
| U6 | How can I reflect on things with my family? | 1 | no |

## GTEBaseENV15 — chunk 300/40 · ctx off · dim native

- Run timestamp: 2026-07-25T22:16:41Z
- Build duration: 809.1 s
- Build window: 2026-07-25T22:03:05Z → 2026-07-25T22:16:34Z
- Build peak RSS: 784.8 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.746 |
| Recall@5 (all) | 0.679 |
| Recall@10 (any) | 0.839 |
| Recall@10 (all) | 0.780 |
| MRR | 0.655 |
| FP-rate@5 | 0.253 |
| Correct-heading | 0.708 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.519 | 0.630 | 0.486 | n/a |
| heading | 24 | 0.708 | 0.875 | 0.641 | 0.708 |
| code-fragment | 14 | 0.714 | 0.714 | 0.643 | n/a |
| multi-note | 18 | 0.944 | 1.000 | 0.739 | n/a |
| exploratory | 17 | 0.706 | 0.882 | 0.556 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 0.944 | n/a |
| staleness | 7 | 0.429 | 0.571 | 0.276 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.706 | 0.824 | 0.602 | 0.737 |
| realistic | 33 | 0.848 | 0.879 | 0.791 | 0.600 |
| diagnostic | 7 | 0.429 | 0.571 | 0.276 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | 10 | no |
| C2 | which box handles name resolution at my parents' place | 9 | no |
| C3 | where does the long-running autonomous agent live | — | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | — | no |
| C5 | where are the offsite copies of my git repositories kept | — | no |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | — | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | — | no |
| C10 | the beans with the citrus and floral notes | 6 | no |
| C11 | who should make decisions on my behalf if I am incapacitated | 1 | yes |
| C12 | which of my data would actually hurt to lose | 4 | no |
| C13 | what should be switched off rather than kept running after I am gone | — | no |
| C14 | something to get my dad | 1 | yes |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | — | — |
| H5 | how are the media drives laid out | 1 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 1 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | 2 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 1 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 7 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 1 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 1 | — |
| F7 | 21116 udp forward | 2 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | — | no |
| F10 | vm.swappiness=10 | — | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | 1 | no |
| F13 | trivy-fleet-audit.timer | 2 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 1 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | no |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 3 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 3 | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 2 | no |
| M6 | what should I use to build an interface that runs in the terminal | 7 | no |
| M7 | getting the car's charge level onto my dashboards | 1 | no |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | no |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 2 | no |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | 2 | yes |
| M14 | where do I keep track of which bags I have already bought | 2 | no |
| X1 | what should we do this weekend | 1 | no |
| X2 | something to put on tonight | — | yes |
| X3 | I want to buy something nice for the flat | 2 | yes |
| X4 | what am I meant to be reading | 9 | no |
| X5 | an idea I could actually sit down and build | 6 | no |
| X6 | keeping the machines patched and scanned for holes | 7 | no |
| X7 | how do I handle people I find hard to deal with | 2 | yes |
| X8 | the general routine for looking after the indoor plants | 2 | yes |
| X9 | how am I going about picking up the language | 3 | no |
| X10 | what should I print next | — | no |
| X11 | ways to give an assistant a memory that persists | 1 | no |
| X12 | how would I find out a disk is dying before it takes something with it | 5 | no |
| X13 | the rules for keeping these notes tidy | 1 | no |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | yes |
| X16 | how are we going to handle solids when the time comes | 1 | no |
| X17 | cheaper ways to rent compute | 1 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 2 | no |
| H12 | what runs overnight, hour by hour | 10 | yes |
| H13 | what happens if the key server is unreachable when a machine reboots | 1 | yes |
| H14 | which of the two feeds should I actually point the indexer at | 1 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | yes |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 3 | no |
| H18 | which guest was actually filling up the backup store | 6 | no |
| H19 | how do I get an off-site agent talking again after its key drifts | 7 | yes |
| H20 | what has changed on the little N100 machine lately | 2 | no |
| H21 | which packages did I deliberately tell the bot to leave alone | — | yes |
| N1 | Kokuyo Campus notebook buying notes | 1 | no |
| C16 | which of the plants gets watered on a fixed weekly schedule | — | yes |
| C17 | the one I should test with a finger instead of watering to a routine | — | no |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | — | no |
| C21 | how many kicks should I feel in two hours before ringing someone | 1 | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 1 | no |
| C24 | who do I call when a pipe leaks | 2 | no |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | 10 | yes |
| S2 | what does network-wide DNS filtering run on | — | no |
| S3 | how do I keep homelab secrets encrypted at rest | — | yes |
| S4 | how are container image updates automated | — | yes |
| S5 | how is the browser terminal exposed | 2 | no |
| S6 | the script that sets up my shell on a freshly built machine | 3 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | — | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 2 | — |
| D14 | Where is the inventory of all my assets and systems? | 2 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 2 | — |
| U2 | I'm looking for a new smell for the house | 1 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 1 | no |
| U5 | I am travelling by plane with the baby | 1 | no |
| U6 | How can I reflect on things with my family? | 1 | no |

## GTEBaseENV15 — chunk 450/50 · ctx on · dim native

- Run timestamp: 2026-07-25T22:31:45Z
- Build duration: 896.0 s
- Build window: 2026-07-25T22:16:42Z → 2026-07-25T22:31:38Z
- Build peak RSS: 798.0 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.763 |
| Recall@5 (all) | 0.704 |
| Recall@10 (any) | 0.839 |
| Recall@10 (all) | 0.791 |
| MRR | 0.673 |
| FP-rate@5 | 0.277 |
| Correct-heading | 0.875 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.593 | 0.630 | 0.452 | n/a |
| heading | 24 | 0.833 | 0.958 | 0.776 | 0.875 |
| code-fragment | 14 | 0.643 | 0.714 | 0.530 | n/a |
| multi-note | 18 | 0.833 | 0.944 | 0.686 | n/a |
| exploratory | 17 | 0.706 | 0.824 | 0.639 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 1.000 | n/a |
| staleness | 7 | 0.429 | 0.571 | 0.278 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.729 | 0.812 | 0.615 | 0.895 |
| realistic | 33 | 0.848 | 0.909 | 0.823 | 0.800 |
| diagnostic | 7 | 0.429 | 0.571 | 0.278 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | — | no |
| C2 | which box handles name resolution at my parents' place | 4 | no |
| C3 | where does the long-running autonomous agent live | — | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 5 | no |
| C5 | where are the offsite copies of my git repositories kept | — | no |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | — | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | — | no |
| C10 | the beans with the citrus and floral notes | 4 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 6 | yes |
| C12 | which of my data would actually hurt to lose | — | no |
| C13 | what should be switched off rather than kept running after I am gone | — | yes |
| C14 | something to get my dad | 1 | no |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | 1 | — |
| H5 | how are the media drives laid out | 1 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 1 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | 1 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 1 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 6 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 1 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 6 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 4 | — |
| F7 | 21116 udp forward | 2 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 2 | no |
| F10 | vm.swappiness=10 | — | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | — | no |
| F13 | trivy-fleet-audit.timer | 1 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 1 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | no |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 1 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 2 | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | — | no |
| M7 | getting the car's charge level onto my dashboards | 1 | no |
| M8 | the wall shades we settled on and their fan-deck codes | 3 | no |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 7 | yes |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | 2 | no |
| M14 | where do I keep track of which bags I have already bought | 6 | no |
| X1 | what should we do this weekend | 1 | no |
| X2 | something to put on tonight | — | no |
| X3 | I want to buy something nice for the flat | 4 | yes |
| X4 | what am I meant to be reading | 1 | yes |
| X5 | an idea I could actually sit down and build | 9 | no |
| X6 | keeping the machines patched and scanned for holes | — | yes |
| X7 | how do I handle people I find hard to deal with | 1 | yes |
| X8 | the general routine for looking after the indoor plants | 3 | yes |
| X9 | how am I going about picking up the language | 1 | no |
| X10 | what should I print next | — | no |
| X11 | ways to give an assistant a memory that persists | 1 | no |
| X12 | how would I find out a disk is dying before it takes something with it | 6 | no |
| X13 | the rules for keeping these notes tidy | 1 | no |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | yes |
| X16 | how are we going to handle solids when the time comes | 1 | no |
| X17 | cheaper ways to rent compute | 1 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 10 | no |
| H12 | what runs overnight, hour by hour | 1 | no |
| H13 | what happens if the key server is unreachable when a machine reboots | 2 | no |
| H14 | which of the two feeds should I actually point the indexer at | 1 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 4 | no |
| H18 | which guest was actually filling up the backup store | 9 | no |
| H19 | how do I get an off-site agent talking again after its key drifts | 1 | no |
| H20 | what has changed on the little N100 machine lately | 1 | yes |
| H21 | which packages did I deliberately tell the bot to leave alone | — | yes |
| N1 | Kokuyo Campus notebook buying notes | 1 | no |
| C16 | which of the plants gets watered on a fixed weekly schedule | — | yes |
| C17 | the one I should test with a finger instead of watering to a routine | 2 | yes |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 3 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | — | no |
| C21 | how many kicks should I feel in two hours before ringing someone | 1 | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 1 | yes |
| C24 | who do I call when a pipe leaks | 2 | no |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | — | yes |
| S2 | what does network-wide DNS filtering run on | — | no |
| S3 | how do I keep homelab secrets encrypted at rest | 9 | yes |
| S4 | how are container image updates automated | — | yes |
| S5 | how is the browser terminal exposed | 3 | no |
| S6 | the script that sets up my shell on a freshly built machine | 2 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 1 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 2 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 2 | — |
| U2 | I'm looking for a new smell for the house | 1 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 5 | no |
| U5 | I am travelling by plane with the baby | 1 | yes |
| U6 | How can I reflect on things with my family? | 1 | no |

## GTEBaseENV15 — chunk 450/50 · ctx off · dim native

- Run timestamp: 2026-07-25T22:45:14Z
- Build duration: 800.6 s
- Build window: 2026-07-25T22:31:47Z → 2026-07-25T22:45:07Z
- Build peak RSS: 777.8 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.805 |
| Recall@5 (all) | 0.734 |
| Recall@10 (any) | 0.864 |
| Recall@10 (all) | 0.812 |
| MRR | 0.655 |
| FP-rate@5 | 0.253 |
| Correct-heading | 0.750 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.630 | 0.704 | 0.509 | n/a |
| heading | 24 | 0.750 | 0.833 | 0.605 | 0.750 |
| code-fragment | 14 | 0.643 | 0.786 | 0.529 | n/a |
| multi-note | 18 | 1.000 | 1.000 | 0.729 | n/a |
| exploratory | 17 | 0.882 | 0.941 | 0.649 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 0.972 | n/a |
| staleness | 7 | 0.571 | 0.571 | 0.333 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.788 | 0.871 | 0.609 | 0.789 |
| realistic | 33 | 0.848 | 0.848 | 0.774 | 0.600 |
| diagnostic | 7 | 0.571 | 0.571 | 0.333 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | — | no |
| C2 | which box handles name resolution at my parents' place | 6 | no |
| C3 | where does the long-running autonomous agent live | — | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 5 | no |
| C5 | where are the offsite copies of my git repositories kept | — | no |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | — | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | — | no |
| C10 | the beans with the citrus and floral notes | 5 | no |
| C11 | who should make decisions on my behalf if I am incapacitated | 1 | yes |
| C12 | which of my data would actually hurt to lose | 3 | no |
| C13 | what should be switched off rather than kept running after I am gone | 7 | no |
| C14 | something to get my dad | 1 | no |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | — | — |
| H5 | how are the media drives laid out | 1 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 3 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | 1 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 2 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 3 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 1 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | 8 | — |
| F5 | ssh -N -L 1455:localhost:1455 | — | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 3 | — |
| F7 | 21116 udp forward | 3 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 9 | no |
| F10 | vm.swappiness=10 | — | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | 2 | no |
| F13 | trivy-fleet-audit.timer | 1 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 1 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | no |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 2 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 2 | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | 5 | no |
| M7 | getting the car's charge level onto my dashboards | 1 | no |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | no |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 3 | no |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | 2 | yes |
| M14 | where do I keep track of which bags I have already bought | 2 | no |
| X1 | what should we do this weekend | 1 | no |
| X2 | something to put on tonight | — | no |
| X3 | I want to buy something nice for the flat | 2 | yes |
| X4 | what am I meant to be reading | 5 | no |
| X5 | an idea I could actually sit down and build | 2 | no |
| X6 | keeping the machines patched and scanned for holes | 5 | no |
| X7 | how do I handle people I find hard to deal with | 2 | yes |
| X8 | the general routine for looking after the indoor plants | 2 | yes |
| X9 | how am I going about picking up the language | 2 | no |
| X10 | what should I print next | 8 | no |
| X11 | ways to give an assistant a memory that persists | 1 | no |
| X12 | how would I find out a disk is dying before it takes something with it | 1 | no |
| X13 | the rules for keeping these notes tidy | 1 | yes |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | yes |
| X16 | how are we going to handle solids when the time comes | 1 | no |
| X17 | cheaper ways to rent compute | 1 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 1 | no |
| H12 | what runs overnight, hour by hour | 6 | yes |
| H13 | what happens if the key server is unreachable when a machine reboots | 2 | yes |
| H14 | which of the two feeds should I actually point the indexer at | 1 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 5 | no |
| H18 | which guest was actually filling up the backup store | 7 | no |
| H19 | how do I get an off-site agent talking again after its key drifts | 3 | yes |
| H20 | what has changed on the little N100 machine lately | — | no |
| H21 | which packages did I deliberately tell the bot to leave alone | — | no |
| N1 | Kokuyo Campus notebook buying notes | 1 | no |
| C16 | which of the plants gets watered on a fixed weekly schedule | — | yes |
| C17 | the one I should test with a finger instead of watering to a routine | 5 | yes |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | — | no |
| C21 | how many kicks should I feel in two hours before ringing someone | 1 | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 1 | yes |
| C24 | who do I call when a pipe leaks | 2 | yes |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | 2 | yes |
| S2 | what does network-wide DNS filtering run on | — | no |
| S3 | how do I keep homelab secrets encrypted at rest | — | yes |
| S4 | how are container image updates automated | — | yes |
| S5 | how is the browser terminal exposed | 2 | no |
| S6 | the script that sets up my shell on a freshly built machine | 3 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | — | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 2 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 4 | — |
| U2 | I'm looking for a new smell for the house | 1 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 3 | no |
| U5 | I am travelling by plane with the baby | 1 | no |
| U6 | How can I reflect on things with my family? | 1 | no |

## GTEBaseENV15 — chunk 800/50 · ctx on · dim native

- Run timestamp: 2026-07-25T23:03:08Z
- Build duration: 1060.6 s
- Build window: 2026-07-25T22:45:16Z → 2026-07-25T23:02:56Z
- Build peak RSS: 956.6 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.814 |
| Recall@5 (all) | 0.746 |
| Recall@10 (any) | 0.881 |
| Recall@10 (all) | 0.825 |
| MRR | 0.700 |
| FP-rate@5 | 0.301 |
| Correct-heading | 0.750 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.667 | 0.667 | 0.488 | n/a |
| heading | 24 | 0.917 | 0.958 | 0.828 | 0.750 |
| code-fragment | 14 | 0.643 | 0.786 | 0.531 | n/a |
| multi-note | 18 | 0.889 | 1.000 | 0.737 | n/a |
| exploratory | 17 | 0.765 | 0.941 | 0.641 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 1.000 | n/a |
| staleness | 7 | 0.429 | 0.857 | 0.344 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.788 | 0.847 | 0.640 | 0.842 |
| realistic | 33 | 0.879 | 0.970 | 0.855 | 0.400 |
| diagnostic | 7 | 0.429 | 0.857 | 0.344 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | — | no |
| C2 | which box handles name resolution at my parents' place | 1 | no |
| C3 | where does the long-running autonomous agent live | — | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 4 | no |
| C5 | where are the offsite copies of my git repositories kept | — | no |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | — | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | — | no |
| C10 | the beans with the citrus and floral notes | 4 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 4 | yes |
| C12 | which of my data would actually hurt to lose | 3 | no |
| C13 | what should be switched off rather than kept running after I am gone | — | no |
| C14 | something to get my dad | 1 | yes |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | 1 | — |
| H5 | how are the media drives laid out | 1 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 1 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | 1 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 2 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 1 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 1 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 3 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 3 | — |
| F7 | 21116 udp forward | 2 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | — | no |
| F10 | vm.swappiness=10 | 8 | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | 7 | no |
| F13 | trivy-fleet-audit.timer | 1 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 1 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | yes |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 1 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 2 | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | 9 | no |
| M7 | getting the car's charge level onto my dashboards | 1 | no |
| M8 | the wall shades we settled on and their fan-deck codes | 2 | no |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 8 | yes |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | 2 | no |
| M14 | where do I keep track of which bags I have already bought | 5 | no |
| X1 | what should we do this weekend | 1 | no |
| X2 | something to put on tonight | — | no |
| X3 | I want to buy something nice for the flat | 4 | yes |
| X4 | what am I meant to be reading | 1 | yes |
| X5 | an idea I could actually sit down and build | 7 | no |
| X6 | keeping the machines patched and scanned for holes | 4 | no |
| X7 | how do I handle people I find hard to deal with | 1 | yes |
| X8 | the general routine for looking after the indoor plants | 2 | yes |
| X9 | how am I going about picking up the language | 2 | no |
| X10 | what should I print next | 9 | no |
| X11 | ways to give an assistant a memory that persists | 1 | no |
| X12 | how would I find out a disk is dying before it takes something with it | 7 | no |
| X13 | the rules for keeping these notes tidy | 1 | no |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | yes |
| X16 | how are we going to handle solids when the time comes | 1 | no |
| X17 | cheaper ways to rent compute | 1 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 5 | no |
| H12 | what runs overnight, hour by hour | 1 | no |
| H13 | what happens if the key server is unreachable when a machine reboots | 2 | yes |
| H14 | which of the two feeds should I actually point the indexer at | 1 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 2 | no |
| H18 | which guest was actually filling up the backup store | 6 | no |
| H19 | how do I get an off-site agent talking again after its key drifts | 1 | no |
| H20 | what has changed on the little N100 machine lately | 1 | yes |
| H21 | which packages did I deliberately tell the bot to leave alone | — | yes |
| N1 | Kokuyo Campus notebook buying notes | 1 | no |
| C16 | which of the plants gets watered on a fixed weekly schedule | — | yes |
| C17 | the one I should test with a finger instead of watering to a routine | 4 | yes |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 3 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | — | no |
| C21 | how many kicks should I feel in two hours before ringing someone | 1 | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 1 | yes |
| C24 | who do I call when a pipe leaks | 2 | yes |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | 10 | yes |
| S2 | what does network-wide DNS filtering run on | — | no |
| S3 | how do I keep homelab secrets encrypted at rest | 7 | yes |
| S4 | how are container image updates automated | 6 | yes |
| S5 | how is the browser terminal exposed | 2 | no |
| S6 | the script that sets up my shell on a freshly built machine | 2 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 1 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 1 | — |
| U2 | I'm looking for a new smell for the house | 1 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 3 | no |
| U5 | I am travelling by plane with the baby | 1 | yes |
| U6 | How can I reflect on things with my family? | 1 | no |

## GTEBaseENV15 — chunk 800/50 · ctx off · dim native

- Run timestamp: 2026-07-25T23:18:39Z
- Build duration: 921.6 s
- Build window: 2026-07-25T23:03:10Z → 2026-07-25T23:18:32Z
- Build peak RSS: 928.2 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.831 |
| Recall@5 (all) | 0.769 |
| Recall@10 (any) | 0.890 |
| Recall@10 (all) | 0.857 |
| MRR | 0.694 |
| FP-rate@5 | 0.253 |
| Correct-heading | 0.625 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.667 | 0.741 | 0.556 | n/a |
| heading | 24 | 0.833 | 0.917 | 0.633 | 0.625 |
| code-fragment | 14 | 0.714 | 0.857 | 0.630 | n/a |
| multi-note | 18 | 0.944 | 1.000 | 0.763 | n/a |
| exploratory | 17 | 0.882 | 0.882 | 0.716 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 0.944 | n/a |
| staleness | 7 | 0.429 | 0.714 | 0.395 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.812 | 0.871 | 0.653 | 0.737 |
| realistic | 33 | 0.879 | 0.939 | 0.799 | 0.200 |
| diagnostic | 7 | 0.429 | 0.714 | 0.395 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | — | no |
| C2 | which box handles name resolution at my parents' place | 2 | no |
| C3 | where does the long-running autonomous agent live | — | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 4 | no |
| C5 | where are the offsite copies of my git repositories kept | — | no |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | — | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | — | no |
| C10 | the beans with the citrus and floral notes | 4 | no |
| C11 | who should make decisions on my behalf if I am incapacitated | 1 | yes |
| C12 | which of my data would actually hurt to lose | 1 | no |
| C13 | what should be switched off rather than kept running after I am gone | 4 | no |
| C14 | something to get my dad | 1 | no |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | — | — |
| H5 | how are the media drives laid out | 1 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 3 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | 1 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 1 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 4 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 1 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | 8 | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 1 | — |
| F7 | 21116 udp forward | 3 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | — | no |
| F10 | vm.swappiness=10 | 9 | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | 4 | no |
| F13 | trivy-fleet-audit.timer | 1 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 1 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 2 | no |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 1 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 2 | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | 7 | no |
| M7 | getting the car's charge level onto my dashboards | 1 | no |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | no |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 3 | yes |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | 2 | yes |
| M14 | where do I keep track of which bags I have already bought | 2 | no |
| X1 | what should we do this weekend | 1 | no |
| X2 | something to put on tonight | — | no |
| X3 | I want to buy something nice for the flat | 2 | yes |
| X4 | what am I meant to be reading | 2 | no |
| X5 | an idea I could actually sit down and build | 3 | no |
| X6 | keeping the machines patched and scanned for holes | 3 | yes |
| X7 | how do I handle people I find hard to deal with | 1 | yes |
| X8 | the general routine for looking after the indoor plants | 1 | yes |
| X9 | how am I going about picking up the language | 1 | no |
| X10 | what should I print next | 2 | no |
| X11 | ways to give an assistant a memory that persists | 1 | no |
| X12 | how would I find out a disk is dying before it takes something with it | — | no |
| X13 | the rules for keeping these notes tidy | 1 | no |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | yes |
| X16 | how are we going to handle solids when the time comes | 1 | no |
| X17 | cheaper ways to rent compute | 1 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 5 | no |
| H12 | what runs overnight, hour by hour | 3 | no |
| H13 | what happens if the key server is unreachable when a machine reboots | 8 | yes |
| H14 | which of the two feeds should I actually point the indexer at | 1 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 5 | no |
| H18 | which guest was actually filling up the backup store | 3 | no |
| H19 | how do I get an off-site agent talking again after its key drifts | 1 | no |
| H20 | what has changed on the little N100 machine lately | 4 | yes |
| H21 | which packages did I deliberately tell the bot to leave alone | — | no |
| N1 | Kokuyo Campus notebook buying notes | 1 | no |
| C16 | which of the plants gets watered on a fixed weekly schedule | 8 | yes |
| C17 | the one I should test with a finger instead of watering to a routine | 8 | yes |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | — | no |
| C21 | how many kicks should I feel in two hours before ringing someone | 1 | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 1 | yes |
| C24 | who do I call when a pipe leaks | 2 | yes |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | 7 | yes |
| S2 | what does network-wide DNS filtering run on | — | no |
| S3 | how do I keep homelab secrets encrypted at rest | — | yes |
| S4 | how are container image updates automated | 8 | yes |
| S5 | how is the browser terminal exposed | 1 | no |
| S6 | the script that sets up my shell on a freshly built machine | 2 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 6 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 2 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 2 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 1 | — |
| U2 | I'm looking for a new smell for the house | 1 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 4 | no |
| U5 | I am travelling by plane with the baby | 1 | no |
| U6 | How can I reflect on things with my family? | 1 | no |

## NomicEmbedTextV2Moe — chunk 300/40 · ctx on · dim native

- Run timestamp: 2026-07-26T00:10:17Z
- Build duration: 3041.4 s
- Build window: 2026-07-25T23:18:47Z → 2026-07-26T00:09:29Z
- Build peak RSS: 3639.2 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.831 |
| Recall@5 (all) | 0.756 |
| Recall@10 (any) | 0.907 |
| Recall@10 (all) | 0.833 |
| MRR | 0.738 |
| FP-rate@5 | 0.229 |
| Correct-heading | 0.833 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.741 | 0.889 | 0.609 | n/a |
| heading | 24 | 0.833 | 0.917 | 0.734 | 0.833 |
| code-fragment | 14 | 0.786 | 0.786 | 0.786 | n/a |
| multi-note | 18 | 0.833 | 0.944 | 0.759 | n/a |
| exploratory | 17 | 0.824 | 0.882 | 0.608 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 1.000 | n/a |
| staleness | 7 | 0.429 | 0.429 | 0.262 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.788 | 0.894 | 0.665 | 0.842 |
| realistic | 33 | 0.939 | 0.939 | 0.924 | 0.800 |
| diagnostic | 7 | 0.429 | 0.429 | 0.262 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | 7 | no |
| C2 | which box handles name resolution at my parents' place | 1 | no |
| C3 | where does the long-running autonomous agent live | 4 | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 6 | no |
| C5 | where are the offsite copies of my git repositories kept | 4 | no |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | 1 | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | 6 | no |
| C10 | the beans with the citrus and floral notes | 2 | no |
| C11 | who should make decisions on my behalf if I am incapacitated | 3 | no |
| C12 | which of my data would actually hurt to lose | 2 | no |
| C13 | what should be switched off rather than kept running after I am gone | 8 | no |
| C14 | something to get my dad | 1 | yes |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | 1 | — |
| H5 | how are the media drives laid out | 2 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 3 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | — | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 1 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 1 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 1 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 1 | — |
| F7 | 21116 udp forward | 1 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 1 | no |
| F10 | vm.swappiness=10 | — | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | 1 | no |
| F13 | trivy-fleet-audit.timer | 1 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 1 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | no |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 6 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 2 | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | — | no |
| M7 | getting the car's charge level onto my dashboards | 1 | no |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | yes |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 2 | no |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | 1 | yes |
| M14 | where do I keep track of which bags I have already bought | 6 | no |
| X1 | what should we do this weekend | 2 | yes |
| X2 | something to put on tonight | — | no |
| X3 | I want to buy something nice for the flat | 2 | yes |
| X4 | what am I meant to be reading | 1 | yes |
| X5 | an idea I could actually sit down and build | 2 | no |
| X6 | keeping the machines patched and scanned for holes | 1 | no |
| X7 | how do I handle people I find hard to deal with | 3 | yes |
| X8 | the general routine for looking after the indoor plants | 3 | yes |
| X9 | how am I going about picking up the language | 1 | no |
| X10 | what should I print next | 2 | no |
| X11 | ways to give an assistant a memory that persists | 1 | no |
| X12 | how would I find out a disk is dying before it takes something with it | — | no |
| X13 | the rules for keeping these notes tidy | 6 | no |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | no |
| X16 | how are we going to handle solids when the time comes | 2 | no |
| X17 | cheaper ways to rent compute | 1 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 1 | no |
| H12 | what runs overnight, hour by hour | 1 | yes |
| H13 | what happens if the key server is unreachable when a machine reboots | 6 | no |
| H14 | which of the two feeds should I actually point the indexer at | 1 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 1 | no |
| H18 | which guest was actually filling up the backup store | — | no |
| H19 | how do I get an off-site agent talking again after its key drifts | 1 | no |
| H20 | what has changed on the little N100 machine lately | 9 | no |
| H21 | which packages did I deliberately tell the bot to leave alone | 4 | no |
| N1 | Kokuyo Campus notebook buying notes | 1 | no |
| C16 | which of the plants gets watered on a fixed weekly schedule | 1 | no |
| C17 | the one I should test with a finger instead of watering to a routine | — | no |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | 2 | yes |
| C21 | how many kicks should I feel in two hours before ringing someone | — | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 1 | yes |
| C24 | who do I call when a pipe leaks | 2 | no |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | no |
| S1 | how are the DAS shares mounted for the media stack | — | yes |
| S2 | what does network-wide DNS filtering run on | — | no |
| S3 | how do I keep homelab secrets encrypted at rest | 2 | yes |
| S4 | how are container image updates automated | — | yes |
| S5 | how is the browser terminal exposed | — | no |
| S6 | the script that sets up my shell on a freshly built machine | 3 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 4 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 1 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 1 | — |
| U2 | I'm looking for a new smell for the house | 3 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 1 | yes |
| U5 | I am travelling by plane with the baby | 1 | no |
| U6 | How can I reflect on things with my family? | 1 | yes |

## NomicEmbedTextV2Moe — chunk 300/40 · ctx off · dim native

- Run timestamp: 2026-07-26T01:01:14Z
- Build duration: 2996.4 s
- Build window: 2026-07-26T00:10:22Z → 2026-07-26T01:00:18Z
- Build peak RSS: 3639.1 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.856 |
| Recall@5 (all) | 0.780 |
| Recall@10 (any) | 0.881 |
| Recall@10 (all) | 0.822 |
| MRR | 0.733 |
| FP-rate@5 | 0.301 |
| Correct-heading | 0.583 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.852 | 0.889 | 0.652 | n/a |
| heading | 24 | 0.708 | 0.750 | 0.634 | 0.583 |
| code-fragment | 14 | 0.786 | 0.786 | 0.750 | n/a |
| multi-note | 18 | 0.944 | 1.000 | 0.796 | n/a |
| exploratory | 17 | 0.882 | 0.882 | 0.637 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 1.000 | n/a |
| staleness | 7 | 0.571 | 0.714 | 0.514 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.835 | 0.859 | 0.671 | 0.579 |
| realistic | 33 | 0.909 | 0.939 | 0.894 | 0.600 |
| diagnostic | 7 | 0.571 | 0.714 | 0.514 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | 3 | no |
| C2 | which box handles name resolution at my parents' place | 1 | no |
| C3 | where does the long-running autonomous agent live | 3 | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 2 | no |
| C5 | where are the offsite copies of my git repositories kept | 2 | no |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | 1 | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | 2 | no |
| C10 | the beans with the citrus and floral notes | 2 | no |
| C11 | who should make decisions on my behalf if I am incapacitated | 2 | no |
| C12 | which of my data would actually hurt to lose | 3 | no |
| C13 | what should be switched off rather than kept running after I am gone | 9 | no |
| C14 | something to get my dad | 1 | yes |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | — | — |
| H5 | how are the media drives laid out | 1 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | — | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | 1 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 4 | — |
| H10 | what were the main design decisions behind the deployment pipeline | — | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 1 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 1 | — |
| F7 | 21116 udp forward | 1 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 1 | no |
| F10 | vm.swappiness=10 | — | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | yes |
| F12 | mnt-tmvol.mount | 1 | no |
| F13 | trivy-fleet-audit.timer | 2 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 1 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | no |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 2 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 3 | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | 2 | no |
| M7 | getting the car's charge level onto my dashboards | 1 | no |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | yes |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | yes |
| M11 | preparing for the maternity nurse visits | 6 | no |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | 1 | yes |
| M14 | where do I keep track of which bags I have already bought | 3 | no |
| X1 | what should we do this weekend | 2 | yes |
| X2 | something to put on tonight | — | yes |
| X3 | I want to buy something nice for the flat | 1 | yes |
| X4 | what am I meant to be reading | 1 | yes |
| X5 | an idea I could actually sit down and build | 1 | no |
| X6 | keeping the machines patched and scanned for holes | 1 | no |
| X7 | how do I handle people I find hard to deal with | 4 | yes |
| X8 | the general routine for looking after the indoor plants | 4 | yes |
| X9 | how am I going about picking up the language | 1 | no |
| X10 | what should I print next | 4 | no |
| X11 | ways to give an assistant a memory that persists | 4 | no |
| X12 | how would I find out a disk is dying before it takes something with it | — | no |
| X13 | the rules for keeping these notes tidy | 3 | no |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | no |
| X16 | how are we going to handle solids when the time comes | 1 | no |
| X17 | cheaper ways to rent compute | 1 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 1 | yes |
| H12 | what runs overnight, hour by hour | — | yes |
| H13 | what happens if the key server is unreachable when a machine reboots | 7 | yes |
| H14 | which of the two feeds should I actually point the indexer at | 1 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 1 | no |
| H18 | which guest was actually filling up the backup store | — | no |
| H19 | how do I get an off-site agent talking again after its key drifts | 2 | yes |
| H20 | what has changed on the little N100 machine lately | — | no |
| H21 | which packages did I deliberately tell the bot to leave alone | 1 | no |
| N1 | Kokuyo Campus notebook buying notes | 1 | no |
| C16 | which of the plants gets watered on a fixed weekly schedule | 1 | yes |
| C17 | the one I should test with a finger instead of watering to a routine | — | no |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | 2 | yes |
| C21 | how many kicks should I feel in two hours before ringing someone | — | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 1 | no |
| C24 | who do I call when a pipe leaks | 2 | no |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | — | yes |
| S2 | what does network-wide DNS filtering run on | — | no |
| S3 | how do I keep homelab secrets encrypted at rest | 1 | yes |
| S4 | how are container image updates automated | 10 | yes |
| S5 | how is the browser terminal exposed | 2 | no |
| S6 | the script that sets up my shell on a freshly built machine | 1 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 3 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 1 | — |
| U2 | I'm looking for a new smell for the house | 2 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 1 | no |
| U5 | I am travelling by plane with the baby | 1 | yes |
| U6 | How can I reflect on things with my family? | 1 | no |

## NomicEmbedTextV2Moe — chunk 450/50 · ctx on · dim native

- Run timestamp: 2026-07-26T01:50:50Z
- Build duration: 2924.5 s
- Build window: 2026-07-26T01:01:19Z → 2026-07-26T01:50:04Z
- Build peak RSS: 3639.2 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.881 |
| Recall@5 (all) | 0.799 |
| Recall@10 (any) | 0.890 |
| Recall@10 (all) | 0.820 |
| MRR | 0.768 |
| FP-rate@5 | 0.313 |
| Correct-heading | 0.875 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.815 | 0.815 | 0.659 | n/a |
| heading | 24 | 0.875 | 0.917 | 0.788 | 0.875 |
| code-fragment | 14 | 0.786 | 0.786 | 0.750 | n/a |
| multi-note | 18 | 0.944 | 0.944 | 0.763 | n/a |
| exploratory | 17 | 0.882 | 0.882 | 0.686 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 1.000 | n/a |
| staleness | 7 | 0.429 | 0.429 | 0.429 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.871 | 0.882 | 0.728 | 0.895 |
| realistic | 33 | 0.909 | 0.909 | 0.870 | 0.800 |
| diagnostic | 7 | 0.429 | 0.429 | 0.429 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | — | no |
| C2 | which box handles name resolution at my parents' place | 1 | no |
| C3 | where does the long-running autonomous agent live | 4 | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 1 | no |
| C5 | where are the offsite copies of my git repositories kept | 2 | no |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | — | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | 5 | no |
| C10 | the beans with the citrus and floral notes | 2 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 2 | no |
| C12 | which of my data would actually hurt to lose | 1 | no |
| C13 | what should be switched off rather than kept running after I am gone | 3 | no |
| C14 | something to get my dad | 1 | yes |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | 1 | — |
| H5 | how are the media drives laid out | 1 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 4 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | — | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 1 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 1 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 1 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 1 | — |
| F7 | 21116 udp forward | 1 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 1 | no |
| F10 | vm.swappiness=10 | — | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | 2 | no |
| F13 | trivy-fleet-audit.timer | 1 | no |
| F14 | when does snapraid-scrub.timer actually fire | 1 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | no |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 1 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 1 | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | — | no |
| M7 | getting the car's charge level onto my dashboards | 1 | no |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | yes |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 5 | no |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | 2 | yes |
| M14 | where do I keep track of which bags I have already bought | 5 | no |
| X1 | what should we do this weekend | 2 | yes |
| X2 | something to put on tonight | — | yes |
| X3 | I want to buy something nice for the flat | 2 | yes |
| X4 | what am I meant to be reading | 1 | yes |
| X5 | an idea I could actually sit down and build | 2 | no |
| X6 | keeping the machines patched and scanned for holes | 1 | no |
| X7 | how do I handle people I find hard to deal with | 2 | yes |
| X8 | the general routine for looking after the indoor plants | 3 | yes |
| X9 | how am I going about picking up the language | 1 | no |
| X10 | what should I print next | 1 | no |
| X11 | ways to give an assistant a memory that persists | 1 | no |
| X12 | how would I find out a disk is dying before it takes something with it | — | no |
| X13 | the rules for keeping these notes tidy | 3 | no |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | no |
| X16 | how are we going to handle solids when the time comes | 1 | no |
| X17 | cheaper ways to rent compute | 1 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 1 | yes |
| H12 | what runs overnight, hour by hour | 1 | yes |
| H13 | what happens if the key server is unreachable when a machine reboots | 5 | no |
| H14 | which of the two feeds should I actually point the indexer at | 1 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 1 | no |
| H18 | which guest was actually filling up the backup store | — | no |
| H19 | how do I get an off-site agent talking again after its key drifts | 1 | yes |
| H20 | what has changed on the little N100 machine lately | 8 | no |
| H21 | which packages did I deliberately tell the bot to leave alone | 3 | no |
| N1 | Kokuyo Campus notebook buying notes | 1 | no |
| C16 | which of the plants gets watered on a fixed weekly schedule | 1 | yes |
| C17 | the one I should test with a finger instead of watering to a routine | — | no |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | 1 | yes |
| C21 | how many kicks should I feel in two hours before ringing someone | — | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 1 | yes |
| C24 | who do I call when a pipe leaks | 2 | no |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | — | yes |
| S2 | what does network-wide DNS filtering run on | — | no |
| S3 | how do I keep homelab secrets encrypted at rest | 1 | yes |
| S4 | how are container image updates automated | — | yes |
| S5 | how is the browser terminal exposed | — | no |
| S6 | the script that sets up my shell on a freshly built machine | 1 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 1 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 1 | — |
| U2 | I'm looking for a new smell for the house | 2 | yes |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 3 | yes |
| U5 | I am travelling by plane with the baby | 1 | yes |
| U6 | How can I reflect on things with my family? | 1 | yes |

## NomicEmbedTextV2Moe — chunk 450/50 · ctx off · dim native

- Run timestamp: 2026-07-26T02:50:15Z
- Build duration: 3527.1 s
- Build window: 2026-07-26T01:50:54Z → 2026-07-26T02:49:41Z
- Build peak RSS: 3639.2 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.890 |
| Recall@5 (all) | 0.813 |
| Recall@10 (any) | 0.924 |
| Recall@10 (all) | 0.871 |
| MRR | 0.752 |
| FP-rate@5 | 0.337 |
| Correct-heading | 0.750 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.852 | 0.889 | 0.670 | n/a |
| heading | 24 | 0.833 | 0.958 | 0.755 | 0.750 |
| code-fragment | 14 | 0.786 | 0.786 | 0.714 | n/a |
| multi-note | 18 | 1.000 | 1.000 | 0.780 | n/a |
| exploratory | 17 | 0.882 | 0.882 | 0.618 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 1.000 | n/a |
| staleness | 7 | 0.714 | 0.714 | 0.414 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.859 | 0.906 | 0.686 | 0.789 |
| realistic | 33 | 0.970 | 0.970 | 0.923 | 0.600 |
| diagnostic | 7 | 0.714 | 0.714 | 0.414 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | 7 | no |
| C2 | which box handles name resolution at my parents' place | 1 | no |
| C3 | where does the long-running autonomous agent live | 5 | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 3 | no |
| C5 | where are the offsite copies of my git repositories kept | 1 | no |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | 1 | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | 3 | no |
| C10 | the beans with the citrus and floral notes | 2 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 2 | no |
| C12 | which of my data would actually hurt to lose | 1 | no |
| C13 | what should be switched off rather than kept running after I am gone | 3 | no |
| C14 | something to get my dad | 1 | yes |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 2 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | 4 | — |
| H5 | how are the media drives laid out | 1 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 2 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | 1 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 2 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 1 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 1 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 1 | — |
| F7 | 21116 udp forward | 1 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 1 | no |
| F10 | vm.swappiness=10 | — | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | yes |
| F12 | mnt-tmvol.mount | 1 | no |
| F13 | trivy-fleet-audit.timer | 2 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 2 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | yes |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 1 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 2 | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | 2 | no |
| M7 | getting the car's charge level onto my dashboards | 1 | no |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | yes |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 5 | no |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | 1 | yes |
| M14 | where do I keep track of which bags I have already bought | 3 | no |
| X1 | what should we do this weekend | 2 | yes |
| X2 | something to put on tonight | — | yes |
| X3 | I want to buy something nice for the flat | 2 | yes |
| X4 | what am I meant to be reading | 1 | yes |
| X5 | an idea I could actually sit down and build | 2 | no |
| X6 | keeping the machines patched and scanned for holes | 1 | no |
| X7 | how do I handle people I find hard to deal with | 3 | yes |
| X8 | the general routine for looking after the indoor plants | 3 | yes |
| X9 | how am I going about picking up the language | 1 | no |
| X10 | what should I print next | 2 | no |
| X11 | ways to give an assistant a memory that persists | 3 | no |
| X12 | how would I find out a disk is dying before it takes something with it | — | no |
| X13 | the rules for keeping these notes tidy | 2 | no |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | no |
| X16 | how are we going to handle solids when the time comes | 1 | no |
| X17 | cheaper ways to rent compute | 1 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 1 | yes |
| H12 | what runs overnight, hour by hour | 9 | yes |
| H13 | what happens if the key server is unreachable when a machine reboots | 8 | no |
| H14 | which of the two feeds should I actually point the indexer at | 1 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | yes |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 1 | no |
| H18 | which guest was actually filling up the backup store | — | no |
| H19 | how do I get an off-site agent talking again after its key drifts | 1 | yes |
| H20 | what has changed on the little N100 machine lately | 8 | no |
| H21 | which packages did I deliberately tell the bot to leave alone | 1 | no |
| N1 | Kokuyo Campus notebook buying notes | 1 | no |
| C16 | which of the plants gets watered on a fixed weekly schedule | 1 | yes |
| C17 | the one I should test with a finger instead of watering to a routine | — | no |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | 4 | yes |
| C21 | how many kicks should I feel in two hours before ringing someone | — | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 1 | yes |
| C24 | who do I call when a pipe leaks | 2 | no |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | 5 | yes |
| S2 | what does network-wide DNS filtering run on | — | no |
| S3 | how do I keep homelab secrets encrypted at rest | 2 | yes |
| S4 | how are container image updates automated | — | yes |
| S5 | how is the browser terminal exposed | 5 | no |
| S6 | the script that sets up my shell on a freshly built machine | 1 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 1 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 2 | — |
| U2 | I'm looking for a new smell for the house | 2 | yes |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 2 | no |
| U5 | I am travelling by plane with the baby | 1 | yes |
| U6 | How can I reflect on things with my family? | 1 | no |

## NomicEmbedTextV2Moe — chunk 800/50 · ctx on · dim native

- Run timestamp: 2026-07-26T03:26:02Z
- Build duration: 2112.7 s
- Build window: 2026-07-26T02:50:18Z → 2026-07-26T03:25:31Z
- Build peak RSS: 3639.2 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.864 |
| Recall@5 (all) | 0.802 |
| Recall@10 (any) | 0.924 |
| Recall@10 (all) | 0.865 |
| MRR | 0.771 |
| FP-rate@5 | 0.325 |
| Correct-heading | 0.792 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.815 | 0.889 | 0.652 | n/a |
| heading | 24 | 0.875 | 0.958 | 0.776 | 0.792 |
| code-fragment | 14 | 0.786 | 0.786 | 0.786 | n/a |
| multi-note | 18 | 0.833 | 0.944 | 0.765 | n/a |
| exploratory | 17 | 0.882 | 0.941 | 0.703 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 1.000 | n/a |
| staleness | 7 | 0.429 | 0.714 | 0.462 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.859 | 0.906 | 0.730 | 0.895 |
| realistic | 33 | 0.879 | 0.970 | 0.875 | 0.400 |
| diagnostic | 7 | 0.429 | 0.714 | 0.462 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | — | no |
| C2 | which box handles name resolution at my parents' place | 1 | no |
| C3 | where does the long-running autonomous agent live | 3 | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 2 | no |
| C5 | where are the offsite copies of my git repositories kept | 2 | yes |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | 10 | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | 5 | no |
| C10 | the beans with the citrus and floral notes | 2 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 2 | no |
| C12 | which of my data would actually hurt to lose | 1 | no |
| C13 | what should be switched off rather than kept running after I am gone | 3 | no |
| C14 | something to get my dad | 1 | yes |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | 1 | — |
| H5 | how are the media drives laid out | 1 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 3 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | 7 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 1 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 1 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 1 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 1 | — |
| F7 | 21116 udp forward | 1 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 1 | no |
| F10 | vm.swappiness=10 | — | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | 1 | no |
| F13 | trivy-fleet-audit.timer | 1 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 1 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | no |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 1 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 1 | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | 10 | no |
| M7 | getting the car's charge level onto my dashboards | 1 | no |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | yes |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 2 | no |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | — | yes |
| M14 | where do I keep track of which bags I have already bought | 6 | no |
| X1 | what should we do this weekend | 2 | yes |
| X2 | something to put on tonight | — | yes |
| X3 | I want to buy something nice for the flat | 2 | yes |
| X4 | what am I meant to be reading | 1 | yes |
| X5 | an idea I could actually sit down and build | 1 | no |
| X6 | keeping the machines patched and scanned for holes | 1 | no |
| X7 | how do I handle people I find hard to deal with | 2 | yes |
| X8 | the general routine for looking after the indoor plants | 2 | yes |
| X9 | how am I going about picking up the language | 1 | no |
| X10 | what should I print next | 1 | no |
| X11 | ways to give an assistant a memory that persists | 2 | no |
| X12 | how would I find out a disk is dying before it takes something with it | 8 | no |
| X13 | the rules for keeping these notes tidy | 3 | no |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | no |
| X16 | how are we going to handle solids when the time comes | 1 | no |
| X17 | cheaper ways to rent compute | 1 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 1 | yes |
| H12 | what runs overnight, hour by hour | 1 | no |
| H13 | what happens if the key server is unreachable when a machine reboots | 4 | no |
| H14 | which of the two feeds should I actually point the indexer at | 2 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 1 | no |
| H18 | which guest was actually filling up the backup store | — | yes |
| H19 | how do I get an off-site agent talking again after its key drifts | 1 | no |
| H20 | what has changed on the little N100 machine lately | 4 | no |
| H21 | which packages did I deliberately tell the bot to leave alone | 1 | no |
| N1 | Kokuyo Campus notebook buying notes | 1 | no |
| C16 | which of the plants gets watered on a fixed weekly schedule | 1 | yes |
| C17 | the one I should test with a finger instead of watering to a routine | — | no |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | 1 | yes |
| C21 | how many kicks should I feel in two hours before ringing someone | 8 | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 1 | yes |
| C24 | who do I call when a pipe leaks | 2 | no |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | 9 | yes |
| S2 | what does network-wide DNS filtering run on | — | no |
| S3 | how do I keep homelab secrets encrypted at rest | 1 | yes |
| S4 | how are container image updates automated | 8 | yes |
| S5 | how is the browser terminal exposed | — | no |
| S6 | the script that sets up my shell on a freshly built machine | 1 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 7 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 1 | — |
| U2 | I'm looking for a new smell for the house | 2 | yes |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 2 | yes |
| U5 | I am travelling by plane with the baby | 1 | yes |
| U6 | How can I reflect on things with my family? | 1 | yes |

## NomicEmbedTextV2Moe — chunk 800/50 · ctx off · dim native

- Run timestamp: 2026-07-26T03:54:26Z
- Build duration: 1666.9 s
- Build window: 2026-07-26T03:26:06Z → 2026-07-26T03:53:52Z
- Build peak RSS: 3639.4 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.915 |
| Recall@5 (all) | 0.850 |
| Recall@10 (any) | 0.941 |
| Recall@10 (all) | 0.893 |
| MRR | 0.770 |
| FP-rate@5 | 0.349 |
| Correct-heading | 0.708 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.852 | 0.889 | 0.688 | n/a |
| heading | 24 | 1.000 | 1.000 | 0.732 | 0.708 |
| code-fragment | 14 | 0.786 | 0.786 | 0.750 | n/a |
| multi-note | 18 | 0.944 | 1.000 | 0.753 | n/a |
| exploratory | 17 | 0.882 | 0.941 | 0.742 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 1.000 | n/a |
| staleness | 7 | 0.714 | 0.857 | 0.510 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.894 | 0.929 | 0.722 | 0.842 |
| realistic | 33 | 0.970 | 0.970 | 0.892 | 0.200 |
| diagnostic | 7 | 0.714 | 0.857 | 0.510 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | 6 | no |
| C2 | which box handles name resolution at my parents' place | 1 | no |
| C3 | where does the long-running autonomous agent live | 4 | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 2 | no |
| C5 | where are the offsite copies of my git repositories kept | 1 | yes |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | 2 | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | 3 | no |
| C10 | the beans with the citrus and floral notes | 2 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 2 | no |
| C12 | which of my data would actually hurt to lose | 1 | no |
| C13 | what should be switched off rather than kept running after I am gone | 3 | no |
| C14 | something to get my dad | 1 | yes |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 2 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | 4 | — |
| H5 | how are the media drives laid out | 1 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 3 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | 1 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 3 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 2 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 1 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 1 | — |
| F7 | 21116 udp forward | 2 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 1 | no |
| F10 | vm.swappiness=10 | — | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | 1 | no |
| F13 | trivy-fleet-audit.timer | 1 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 1 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | yes |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 1 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 2 | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | 2 | no |
| M7 | getting the car's charge level onto my dashboards | 1 | no |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | yes |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 5 | no |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | 9 | yes |
| M14 | where do I keep track of which bags I have already bought | 4 | no |
| X1 | what should we do this weekend | 2 | yes |
| X2 | something to put on tonight | — | yes |
| X3 | I want to buy something nice for the flat | 2 | yes |
| X4 | what am I meant to be reading | 1 | yes |
| X5 | an idea I could actually sit down and build | 1 | no |
| X6 | keeping the machines patched and scanned for holes | 1 | no |
| X7 | how do I handle people I find hard to deal with | 2 | yes |
| X8 | the general routine for looking after the indoor plants | 2 | yes |
| X9 | how am I going about picking up the language | 1 | no |
| X10 | what should I print next | 1 | no |
| X11 | ways to give an assistant a memory that persists | 2 | no |
| X12 | how would I find out a disk is dying before it takes something with it | 9 | no |
| X13 | the rules for keeping these notes tidy | 1 | no |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | no |
| X16 | how are we going to handle solids when the time comes | 1 | no |
| X17 | cheaper ways to rent compute | 1 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 1 | yes |
| H12 | what runs overnight, hour by hour | 2 | yes |
| H13 | what happens if the key server is unreachable when a machine reboots | 5 | no |
| H14 | which of the two feeds should I actually point the indexer at | 1 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | yes |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 1 | no |
| H18 | which guest was actually filling up the backup store | 5 | no |
| H19 | how do I get an off-site agent talking again after its key drifts | 1 | no |
| H20 | what has changed on the little N100 machine lately | 4 | no |
| H21 | which packages did I deliberately tell the bot to leave alone | 1 | no |
| N1 | Kokuyo Campus notebook buying notes | 1 | no |
| C16 | which of the plants gets watered on a fixed weekly schedule | 1 | yes |
| C17 | the one I should test with a finger instead of watering to a routine | — | no |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | 1 | yes |
| C21 | how many kicks should I feel in two hours before ringing someone | — | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 1 | yes |
| C24 | who do I call when a pipe leaks | 2 | no |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | 5 | yes |
| S2 | what does network-wide DNS filtering run on | — | no |
| S3 | how do I keep homelab secrets encrypted at rest | 1 | yes |
| S4 | how are container image updates automated | 6 | yes |
| S5 | how is the browser terminal exposed | 5 | no |
| S6 | the script that sets up my shell on a freshly built machine | 1 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 2 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 1 | — |
| U2 | I'm looking for a new smell for the house | 2 | yes |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 2 | yes |
| U5 | I am travelling by plane with the baby | 1 | yes |
| U6 | How can I reflect on things with my family? | 1 | yes |

## NomicEmbedTextV2Moe — chunk 300/40 · ctx on · dim 256

- Run timestamp: 2026-07-26T04:31:09Z
- Build duration: 2170.8 s
- Build window: 2026-07-26T03:54:30Z → 2026-07-26T04:30:40Z
- Build peak RSS: 3639.2 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.771 |
| Recall@5 (all) | 0.701 |
| Recall@10 (any) | 0.839 |
| Recall@10 (all) | 0.775 |
| MRR | 0.672 |
| FP-rate@5 | 0.193 |
| Correct-heading | 0.792 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.630 | 0.741 | 0.499 | n/a |
| heading | 24 | 0.792 | 0.917 | 0.700 | 0.792 |
| code-fragment | 14 | 0.714 | 0.786 | 0.655 | n/a |
| multi-note | 18 | 0.833 | 0.889 | 0.743 | n/a |
| exploratory | 17 | 0.706 | 0.706 | 0.497 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 1.000 | n/a |
| staleness | 7 | 0.429 | 0.571 | 0.378 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.718 | 0.812 | 0.585 | 0.789 |
| realistic | 33 | 0.909 | 0.909 | 0.894 | 0.800 |
| diagnostic | 7 | 0.429 | 0.571 | 0.378 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | — | no |
| C2 | which box handles name resolution at my parents' place | 4 | no |
| C3 | where does the long-running autonomous agent live | 2 | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 8 | no |
| C5 | where are the offsite copies of my git repositories kept | — | no |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | — | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | — | no |
| C10 | the beans with the citrus and floral notes | 4 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 3 | no |
| C12 | which of my data would actually hurt to lose | 2 | no |
| C13 | what should be switched off rather than kept running after I am gone | 7 | no |
| C14 | something to get my dad | 1 | yes |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | 1 | — |
| H5 | how are the media drives laid out | 8 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 1 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | — | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 2 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 1 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 6 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 1 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 1 | — |
| F7 | 21116 udp forward | 2 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 2 | no |
| F10 | vm.swappiness=10 | — | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | 1 | no |
| F13 | trivy-fleet-audit.timer | 1 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 1 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | no |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 4 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | — | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | 1 | no |
| M7 | getting the car's charge level onto my dashboards | 1 | no |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | no |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 2 | no |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | — | no |
| M14 | where do I keep track of which bags I have already bought | 8 | no |
| X1 | what should we do this weekend | 3 | yes |
| X2 | something to put on tonight | — | no |
| X3 | I want to buy something nice for the flat | 3 | yes |
| X4 | what am I meant to be reading | 1 | no |
| X5 | an idea I could actually sit down and build | 5 | no |
| X6 | keeping the machines patched and scanned for holes | 1 | no |
| X7 | how do I handle people I find hard to deal with | 1 | yes |
| X8 | the general routine for looking after the indoor plants | 4 | yes |
| X9 | how am I going about picking up the language | 1 | no |
| X10 | what should I print next | 1 | no |
| X11 | ways to give an assistant a memory that persists | 3 | no |
| X12 | how would I find out a disk is dying before it takes something with it | — | no |
| X13 | the rules for keeping these notes tidy | — | no |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | no |
| X16 | how are we going to handle solids when the time comes | — | no |
| X17 | cheaper ways to rent compute | — | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 1 | no |
| H12 | what runs overnight, hour by hour | 1 | yes |
| H13 | what happens if the key server is unreachable when a machine reboots | 6 | yes |
| H14 | which of the two feeds should I actually point the indexer at | 2 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | — | no |
| H18 | which guest was actually filling up the backup store | 10 | no |
| H19 | how do I get an off-site agent talking again after its key drifts | 1 | no |
| H20 | what has changed on the little N100 machine lately | 5 | yes |
| H21 | which packages did I deliberately tell the bot to leave alone | 5 | no |
| N1 | Kokuyo Campus notebook buying notes | 1 | no |
| C16 | which of the plants gets watered on a fixed weekly schedule | 1 | no |
| C17 | the one I should test with a finger instead of watering to a routine | — | no |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | 9 | no |
| C21 | how many kicks should I feel in two hours before ringing someone | — | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 1 | no |
| C24 | who do I call when a pipe leaks | 4 | no |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | no |
| S1 | how are the DAS shares mounted for the media stack | — | yes |
| S2 | what does network-wide DNS filtering run on | — | no |
| S3 | how do I keep homelab secrets encrypted at rest | 1 | no |
| S4 | how are container image updates automated | 2 | yes |
| S5 | how is the browser terminal exposed | — | no |
| S6 | the script that sets up my shell on a freshly built machine | 7 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 1 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 1 | — |
| U2 | I'm looking for a new smell for the house | 2 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 1 | yes |
| U5 | I am travelling by plane with the baby | 1 | no |
| U6 | How can I reflect on things with my family? | 1 | no |

## NomicEmbedTextV2Moe — chunk 300/40 · ctx off · dim 256

- Run timestamp: 2026-07-26T05:02:07Z
- Build duration: 1822.6 s
- Build window: 2026-07-26T04:31:12Z → 2026-07-26T05:01:34Z
- Build peak RSS: 3639.2 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.814 |
| Recall@5 (all) | 0.741 |
| Recall@10 (any) | 0.864 |
| Recall@10 (all) | 0.808 |
| MRR | 0.646 |
| FP-rate@5 | 0.253 |
| Correct-heading | 0.583 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.704 | 0.778 | 0.530 | n/a |
| heading | 24 | 0.833 | 0.917 | 0.583 | 0.583 |
| code-fragment | 14 | 0.786 | 0.786 | 0.661 | n/a |
| multi-note | 18 | 0.889 | 0.889 | 0.713 | n/a |
| exploratory | 17 | 0.706 | 0.824 | 0.458 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 1.000 | n/a |
| staleness | 7 | 0.571 | 0.857 | 0.542 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.776 | 0.847 | 0.561 | 0.632 |
| realistic | 33 | 0.909 | 0.909 | 0.864 | 0.400 |
| diagnostic | 7 | 0.571 | 0.857 | 0.542 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | — | no |
| C2 | which box handles name resolution at my parents' place | 2 | no |
| C3 | where does the long-running autonomous agent live | 1 | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 6 | no |
| C5 | where are the offsite copies of my git repositories kept | — | no |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | 4 | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | 4 | no |
| C10 | the beans with the citrus and floral notes | 4 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 4 | no |
| C12 | which of my data would actually hurt to lose | 3 | no |
| C13 | what should be switched off rather than kept running after I am gone | 10 | no |
| C14 | something to get my dad | 1 | yes |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | — | — |
| H5 | how are the media drives laid out | 1 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 3 | — |
| H7 | which clients are currently connected to the VPN | 2 | — |
| H8 | what to check first when name resolution breaks | 1 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 3 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 4 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 1 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 1 | — |
| F7 | 21116 udp forward | 4 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 1 | no |
| F10 | vm.swappiness=10 | — | — |
| F11 | telegram-notify@ template unit OnFailure | 2 | yes |
| F12 | mnt-tmvol.mount | 1 | no |
| F13 | trivy-fleet-audit.timer | 2 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 1 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | no |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 4 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | — | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | 1 | no |
| M7 | getting the car's charge level onto my dashboards | 1 | no |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | no |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | yes |
| M11 | preparing for the maternity nurse visits | — | no |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | 3 | no |
| M14 | where do I keep track of which bags I have already bought | 4 | no |
| X1 | what should we do this weekend | 3 | no |
| X2 | something to put on tonight | — | no |
| X3 | I want to buy something nice for the flat | 7 | no |
| X4 | what am I meant to be reading | 1 | no |
| X5 | an idea I could actually sit down and build | 4 | no |
| X6 | keeping the machines patched and scanned for holes | 1 | no |
| X7 | how do I handle people I find hard to deal with | 2 | yes |
| X8 | the general routine for looking after the indoor plants | 5 | yes |
| X9 | how am I going about picking up the language | 1 | no |
| X10 | what should I print next | 6 | no |
| X11 | ways to give an assistant a memory that persists | — | no |
| X12 | how would I find out a disk is dying before it takes something with it | — | no |
| X13 | the rules for keeping these notes tidy | 5 | no |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | no |
| X16 | how are we going to handle solids when the time comes | 2 | no |
| X17 | cheaper ways to rent compute | 2 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 1 | no |
| H12 | what runs overnight, hour by hour | — | yes |
| H13 | what happens if the key server is unreachable when a machine reboots | 9 | yes |
| H14 | which of the two feeds should I actually point the indexer at | 2 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 3 | no |
| H18 | which guest was actually filling up the backup store | 4 | yes |
| H19 | how do I get an off-site agent talking again after its key drifts | 2 | yes |
| H20 | what has changed on the little N100 machine lately | 7 | no |
| H21 | which packages did I deliberately tell the bot to leave alone | 2 | no |
| N1 | Kokuyo Campus notebook buying notes | 1 | no |
| C16 | which of the plants gets watered on a fixed weekly schedule | 1 | yes |
| C17 | the one I should test with a finger instead of watering to a routine | — | no |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | — | no |
| C21 | how many kicks should I feel in two hours before ringing someone | — | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 1 | no |
| C24 | who do I call when a pipe leaks | 5 | no |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | 8 | yes |
| S2 | what does network-wide DNS filtering run on | — | no |
| S3 | how do I keep homelab secrets encrypted at rest | 1 | yes |
| S4 | how are container image updates automated | 6 | yes |
| S5 | how is the browser terminal exposed | 2 | no |
| S6 | the script that sets up my shell on a freshly built machine | 1 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 4 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 2 | — |
| U2 | I'm looking for a new smell for the house | 2 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 1 | yes |
| U5 | I am travelling by plane with the baby | 1 | yes |
| U6 | How can I reflect on things with my family? | 1 | yes |

## NomicEmbedTextV2Moe — chunk 450/50 · ctx on · dim 256

- Run timestamp: 2026-07-26T05:38:17Z
- Build duration: 2133.8 s
- Build window: 2026-07-26T05:02:10Z → 2026-07-26T05:37:44Z
- Build peak RSS: 3639.2 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.754 |
| Recall@5 (all) | 0.696 |
| Recall@10 (any) | 0.864 |
| Recall@10 (all) | 0.800 |
| MRR | 0.664 |
| FP-rate@5 | 0.169 |
| Correct-heading | 0.875 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.704 | 0.741 | 0.541 | n/a |
| heading | 24 | 0.708 | 0.958 | 0.701 | 0.875 |
| code-fragment | 14 | 0.643 | 0.786 | 0.536 | n/a |
| multi-note | 18 | 0.833 | 0.944 | 0.702 | n/a |
| exploratory | 17 | 0.647 | 0.765 | 0.514 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 1.000 | n/a |
| staleness | 7 | 0.571 | 0.714 | 0.520 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.706 | 0.847 | 0.590 | 0.895 |
| realistic | 33 | 0.879 | 0.909 | 0.854 | 0.800 |
| diagnostic | 7 | 0.571 | 0.714 | 0.520 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | — | no |
| C2 | which box handles name resolution at my parents' place | 3 | no |
| C3 | where does the long-running autonomous agent live | 2 | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 3 | no |
| C5 | where are the offsite copies of my git repositories kept | — | no |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | — | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | — | no |
| C10 | the beans with the citrus and floral notes | 4 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 2 | no |
| C12 | which of my data would actually hurt to lose | 1 | no |
| C13 | what should be switched off rather than kept running after I am gone | 4 | no |
| C14 | something to get my dad | 1 | yes |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | 2 | — |
| H5 | how are the media drives laid out | 8 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 1 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | — | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 1 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 1 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 6 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 3 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 2 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 1 | — |
| F7 | 21116 udp forward | 1 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 3 | no |
| F10 | vm.swappiness=10 | — | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | 6 | no |
| F13 | trivy-fleet-audit.timer | 1 | no |
| F14 | when does snapraid-scrub.timer actually fire | 1 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 2 | no |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 2 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 7 | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | 2 | no |
| M7 | getting the car's charge level onto my dashboards | 1 | no |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | no |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 1 | no |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | — | no |
| M14 | where do I keep track of which bags I have already bought | 6 | no |
| X1 | what should we do this weekend | 2 | yes |
| X2 | something to put on tonight | — | no |
| X3 | I want to buy something nice for the flat | 4 | yes |
| X4 | what am I meant to be reading | 1 | no |
| X5 | an idea I could actually sit down and build | 4 | no |
| X6 | keeping the machines patched and scanned for holes | 1 | no |
| X7 | how do I handle people I find hard to deal with | 1 | yes |
| X8 | the general routine for looking after the indoor plants | 2 | yes |
| X9 | how am I going about picking up the language | 1 | no |
| X10 | what should I print next | 1 | no |
| X11 | ways to give an assistant a memory that persists | — | no |
| X12 | how would I find out a disk is dying before it takes something with it | — | no |
| X13 | the rules for keeping these notes tidy | 10 | no |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | no |
| X16 | how are we going to handle solids when the time comes | — | no |
| X17 | cheaper ways to rent compute | 7 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 2 | no |
| H12 | what runs overnight, hour by hour | 1 | no |
| H13 | what happens if the key server is unreachable when a machine reboots | 9 | no |
| H14 | which of the two feeds should I actually point the indexer at | 1 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 7 | no |
| H18 | which guest was actually filling up the backup store | 7 | no |
| H19 | how do I get an off-site agent talking again after its key drifts | 1 | no |
| H20 | what has changed on the little N100 machine lately | 6 | yes |
| H21 | which packages did I deliberately tell the bot to leave alone | 8 | no |
| N1 | Kokuyo Campus notebook buying notes | 1 | no |
| C16 | which of the plants gets watered on a fixed weekly schedule | 1 | no |
| C17 | the one I should test with a finger instead of watering to a routine | — | no |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | 10 | no |
| C21 | how many kicks should I feel in two hours before ringing someone | — | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 1 | no |
| C24 | who do I call when a pipe leaks | 3 | no |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | — | yes |
| S2 | what does network-wide DNS filtering run on | — | no |
| S3 | how do I keep homelab secrets encrypted at rest | 1 | no |
| S4 | how are container image updates automated | 1 | yes |
| S5 | how is the browser terminal exposed | 7 | no |
| S6 | the script that sets up my shell on a freshly built machine | 2 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 1 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 1 | — |
| U2 | I'm looking for a new smell for the house | 2 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 3 | yes |
| U5 | I am travelling by plane with the baby | 1 | no |
| U6 | How can I reflect on things with my family? | 1 | no |

## NomicEmbedTextV2Moe — chunk 450/50 · ctx off · dim 256

- Run timestamp: 2026-07-26T06:11:32Z
- Build duration: 1960.1 s
- Build window: 2026-07-26T05:38:20Z → 2026-07-26T06:11:00Z
- Build peak RSS: 3639.2 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.788 |
| Recall@5 (all) | 0.725 |
| Recall@10 (any) | 0.898 |
| Recall@10 (all) | 0.835 |
| MRR | 0.641 |
| FP-rate@5 | 0.289 |
| Correct-heading | 0.708 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.704 | 0.815 | 0.572 | n/a |
| heading | 24 | 0.750 | 1.000 | 0.605 | 0.708 |
| code-fragment | 14 | 0.714 | 0.714 | 0.607 | n/a |
| multi-note | 18 | 0.833 | 1.000 | 0.647 | n/a |
| exploratory | 17 | 0.765 | 0.824 | 0.445 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 1.000 | n/a |
| staleness | 7 | 0.714 | 0.857 | 0.624 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.753 | 0.871 | 0.553 | 0.737 |
| realistic | 33 | 0.879 | 0.970 | 0.869 | 0.600 |
| diagnostic | 7 | 0.714 | 0.857 | 0.624 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | — | no |
| C2 | which box handles name resolution at my parents' place | 1 | no |
| C3 | where does the long-running autonomous agent live | 1 | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 3 | no |
| C5 | where are the offsite copies of my git repositories kept | — | no |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | 10 | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | 4 | no |
| C10 | the beans with the citrus and floral notes | 4 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 2 | no |
| C12 | which of my data would actually hurt to lose | 2 | no |
| C13 | what should be switched off rather than kept running after I am gone | 6 | no |
| C14 | something to get my dad | 1 | yes |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | 9 | — |
| H5 | how are the media drives laid out | 1 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 2 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | 1 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 2 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 3 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | — | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 1 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 1 | — |
| F7 | 21116 udp forward | 2 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 1 | no |
| F10 | vm.swappiness=10 | — | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | yes |
| F12 | mnt-tmvol.mount | 1 | no |
| F13 | trivy-fleet-audit.timer | 2 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 2 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 5 | yes |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 4 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 7 | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | yes |
| M6 | what should I use to build an interface that runs in the terminal | 2 | no |
| M7 | getting the car's charge level onto my dashboards | 1 | no |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | no |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 7 | no |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | 4 | no |
| M14 | where do I keep track of which bags I have already bought | 6 | no |
| X1 | what should we do this weekend | 2 | yes |
| X2 | something to put on tonight | — | no |
| X3 | I want to buy something nice for the flat | 6 | yes |
| X4 | what am I meant to be reading | 1 | no |
| X5 | an idea I could actually sit down and build | 5 | no |
| X6 | keeping the machines patched and scanned for holes | 1 | no |
| X7 | how do I handle people I find hard to deal with | 2 | yes |
| X8 | the general routine for looking after the indoor plants | 2 | yes |
| X9 | how am I going about picking up the language | 4 | no |
| X10 | what should I print next | 4 | no |
| X11 | ways to give an assistant a memory that persists | — | no |
| X12 | how would I find out a disk is dying before it takes something with it | — | no |
| X13 | the rules for keeping these notes tidy | 5 | no |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | no |
| X16 | how are we going to handle solids when the time comes | 2 | no |
| X17 | cheaper ways to rent compute | 2 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 5 | yes |
| H12 | what runs overnight, hour by hour | 10 | yes |
| H13 | what happens if the key server is unreachable when a machine reboots | 8 | yes |
| H14 | which of the two feeds should I actually point the indexer at | 2 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 6 | no |
| H18 | which guest was actually filling up the backup store | 6 | yes |
| H19 | how do I get an off-site agent talking again after its key drifts | 3 | yes |
| H20 | what has changed on the little N100 machine lately | 7 | no |
| H21 | which packages did I deliberately tell the bot to leave alone | 1 | no |
| N1 | Kokuyo Campus notebook buying notes | 1 | no |
| C16 | which of the plants gets watered on a fixed weekly schedule | 1 | yes |
| C17 | the one I should test with a finger instead of watering to a routine | — | no |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | 10 | yes |
| C21 | how many kicks should I feel in two hours before ringing someone | — | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 1 | no |
| C24 | who do I call when a pipe leaks | 4 | no |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | 6 | yes |
| S2 | what does network-wide DNS filtering run on | — | no |
| S3 | how do I keep homelab secrets encrypted at rest | 1 | yes |
| S4 | how are container image updates automated | 5 | yes |
| S5 | how is the browser terminal exposed | 1 | no |
| S6 | the script that sets up my shell on a freshly built machine | 1 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 3 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 1 | — |
| U2 | I'm looking for a new smell for the house | 2 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 2 | no |
| U5 | I am travelling by plane with the baby | 1 | yes |
| U6 | How can I reflect on things with my family? | 1 | no |

## NomicEmbedTextV2Moe — chunk 800/50 · ctx on · dim 256

- Run timestamp: 2026-07-26T06:39:03Z
- Build duration: 1616.8 s
- Build window: 2026-07-26T06:11:36Z → 2026-07-26T06:38:32Z
- Build peak RSS: 3638.8 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.839 |
| Recall@5 (all) | 0.766 |
| Recall@10 (any) | 0.864 |
| Recall@10 (all) | 0.800 |
| MRR | 0.715 |
| FP-rate@5 | 0.217 |
| Correct-heading | 0.750 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.741 | 0.741 | 0.608 | n/a |
| heading | 24 | 0.917 | 0.958 | 0.785 | 0.750 |
| code-fragment | 14 | 0.714 | 0.714 | 0.595 | n/a |
| multi-note | 18 | 0.944 | 0.944 | 0.733 | n/a |
| exploratory | 17 | 0.706 | 0.824 | 0.563 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 1.000 | n/a |
| staleness | 7 | 0.571 | 0.857 | 0.495 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.812 | 0.847 | 0.651 | 0.895 |
| realistic | 33 | 0.909 | 0.909 | 0.879 | 0.200 |
| diagnostic | 7 | 0.571 | 0.857 | 0.495 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | — | no |
| C2 | which box handles name resolution at my parents' place | 1 | no |
| C3 | where does the long-running autonomous agent live | 1 | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 3 | no |
| C5 | where are the offsite copies of my git repositories kept | — | no |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | — | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | — | no |
| C10 | the beans with the citrus and floral notes | 4 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 2 | no |
| C12 | which of my data would actually hurt to lose | 1 | no |
| C13 | what should be switched off rather than kept running after I am gone | 2 | no |
| C14 | something to get my dad | 1 | yes |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | 2 | — |
| H5 | how are the media drives laid out | 4 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 1 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | — | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 1 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 1 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | — | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 3 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 1 | — |
| F7 | 21116 udp forward | 1 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 2 | no |
| F10 | vm.swappiness=10 | — | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | 2 | no |
| F13 | trivy-fleet-audit.timer | 1 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 1 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 4 | no |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 2 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 4 | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | 1 | no |
| M7 | getting the car's charge level onto my dashboards | 1 | no |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | no |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 1 | no |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | — | no |
| M14 | where do I keep track of which bags I have already bought | 5 | no |
| X1 | what should we do this weekend | 2 | yes |
| X2 | something to put on tonight | — | no |
| X3 | I want to buy something nice for the flat | 4 | no |
| X4 | what am I meant to be reading | 1 | no |
| X5 | an idea I could actually sit down and build | 3 | no |
| X6 | keeping the machines patched and scanned for holes | 1 | no |
| X7 | how do I handle people I find hard to deal with | 1 | yes |
| X8 | the general routine for looking after the indoor plants | 1 | yes |
| X9 | how am I going about picking up the language | 1 | no |
| X10 | what should I print next | 1 | no |
| X11 | ways to give an assistant a memory that persists | 8 | no |
| X12 | how would I find out a disk is dying before it takes something with it | — | no |
| X13 | the rules for keeping these notes tidy | 6 | no |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | no |
| X16 | how are we going to handle solids when the time comes | — | no |
| X17 | cheaper ways to rent compute | 5 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 1 | no |
| H12 | what runs overnight, hour by hour | 1 | no |
| H13 | what happens if the key server is unreachable when a machine reboots | 1 | no |
| H14 | which of the two feeds should I actually point the indexer at | 1 | yes |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 5 | no |
| H18 | which guest was actually filling up the backup store | 2 | yes |
| H19 | how do I get an off-site agent talking again after its key drifts | 1 | no |
| H20 | what has changed on the little N100 machine lately | 7 | no |
| H21 | which packages did I deliberately tell the bot to leave alone | 4 | no |
| N1 | Kokuyo Campus notebook buying notes | 1 | no |
| C16 | which of the plants gets watered on a fixed weekly schedule | 1 | no |
| C17 | the one I should test with a finger instead of watering to a routine | — | no |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | 2 | no |
| C21 | how many kicks should I feel in two hours before ringing someone | — | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 1 | yes |
| C24 | who do I call when a pipe leaks | 3 | no |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | 10 | yes |
| S2 | what does network-wide DNS filtering run on | — | no |
| S3 | how do I keep homelab secrets encrypted at rest | 1 | yes |
| S4 | how are container image updates automated | 5 | yes |
| S5 | how is the browser terminal exposed | 6 | no |
| S6 | the script that sets up my shell on a freshly built machine | 1 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 1 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 1 | — |
| U2 | I'm looking for a new smell for the house | 2 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 2 | yes |
| U5 | I am travelling by plane with the baby | 1 | yes |
| U6 | How can I reflect on things with my family? | 1 | yes |

## NomicEmbedTextV2Moe — chunk 800/50 · ctx off · dim 256

- Run timestamp: 2026-07-26T07:08:39Z
- Build duration: 1736.5 s
- Build window: 2026-07-26T06:39:07Z → 2026-07-26T07:08:03Z
- Build peak RSS: 3638.6 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.847 |
| Recall@5 (all) | 0.789 |
| Recall@10 (any) | 0.898 |
| Recall@10 (all) | 0.839 |
| MRR | 0.691 |
| FP-rate@5 | 0.253 |
| Correct-heading | 0.708 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.778 | 0.815 | 0.631 | n/a |
| heading | 24 | 0.917 | 0.958 | 0.695 | 0.708 |
| code-fragment | 14 | 0.714 | 0.786 | 0.605 | n/a |
| multi-note | 18 | 0.833 | 0.889 | 0.634 | n/a |
| exploratory | 17 | 0.824 | 0.941 | 0.588 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 1.000 | n/a |
| staleness | 7 | 0.571 | 0.857 | 0.544 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.824 | 0.894 | 0.615 | 0.842 |
| realistic | 33 | 0.909 | 0.909 | 0.889 | 0.200 |
| diagnostic | 7 | 0.571 | 0.857 | 0.544 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | — | no |
| C2 | which box handles name resolution at my parents' place | 1 | no |
| C3 | where does the long-running autonomous agent live | 1 | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 4 | no |
| C5 | where are the offsite copies of my git repositories kept | 9 | no |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | — | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | 4 | no |
| C10 | the beans with the citrus and floral notes | 4 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 2 | no |
| C12 | which of my data would actually hurt to lose | 1 | no |
| C13 | what should be switched off rather than kept running after I am gone | 3 | no |
| C14 | something to get my dad | 1 | yes |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 2 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | — | — |
| H5 | how are the media drives laid out | 1 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 1 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | 1 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 5 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 2 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 7 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 2 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 1 | — |
| F7 | 21116 udp forward | 3 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 1 | no |
| F10 | vm.swappiness=10 | — | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | 1 | no |
| F13 | trivy-fleet-audit.timer | 1 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 2 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 4 | yes |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 1 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | — | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 3 | no |
| M6 | what should I use to build an interface that runs in the terminal | 2 | no |
| M7 | getting the car's charge level onto my dashboards | 1 | no |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | no |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 3 | no |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | — | no |
| M14 | where do I keep track of which bags I have already bought | 6 | no |
| X1 | what should we do this weekend | 2 | yes |
| X2 | something to put on tonight | 9 | no |
| X3 | I want to buy something nice for the flat | 5 | no |
| X4 | what am I meant to be reading | 1 | no |
| X5 | an idea I could actually sit down and build | 2 | no |
| X6 | keeping the machines patched and scanned for holes | 1 | no |
| X7 | how do I handle people I find hard to deal with | 1 | yes |
| X8 | the general routine for looking after the indoor plants | 1 | yes |
| X9 | how am I going about picking up the language | 1 | no |
| X10 | what should I print next | 2 | no |
| X11 | ways to give an assistant a memory that persists | 10 | no |
| X12 | how would I find out a disk is dying before it takes something with it | — | no |
| X13 | the rules for keeping these notes tidy | 3 | no |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | no |
| X16 | how are we going to handle solids when the time comes | 4 | no |
| X17 | cheaper ways to rent compute | 2 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 2 | no |
| H12 | what runs overnight, hour by hour | 3 | no |
| H13 | what happens if the key server is unreachable when a machine reboots | 7 | no |
| H14 | which of the two feeds should I actually point the indexer at | 2 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | yes |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 2 | no |
| H18 | which guest was actually filling up the backup store | 4 | yes |
| H19 | how do I get an off-site agent talking again after its key drifts | 1 | no |
| H20 | what has changed on the little N100 machine lately | 4 | no |
| H21 | which packages did I deliberately tell the bot to leave alone | 1 | no |
| N1 | Kokuyo Campus notebook buying notes | 1 | no |
| C16 | which of the plants gets watered on a fixed weekly schedule | 1 | yes |
| C17 | the one I should test with a finger instead of watering to a routine | — | no |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | 1 | yes |
| C21 | how many kicks should I feel in two hours before ringing someone | — | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 1 | yes |
| C24 | who do I call when a pipe leaks | 3 | no |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | 6 | yes |
| S2 | what does network-wide DNS filtering run on | — | no |
| S3 | how do I keep homelab secrets encrypted at rest | 1 | yes |
| S4 | how are container image updates automated | 7 | yes |
| S5 | how is the browser terminal exposed | 2 | no |
| S6 | the script that sets up my shell on a freshly built machine | 1 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 1 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 1 | — |
| U2 | I'm looking for a new smell for the house | 2 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 3 | yes |
| U5 | I am travelling by plane with the baby | 1 | yes |
| U6 | How can I reflect on things with my family? | 1 | yes |

## Qwen3Embedding0_6B — chunk 800/50 · ctx on · dim 512

- Run timestamp: 2026-07-26T09:17:53Z
- Build duration: 5616.8 s
- Build window: 2026-07-26T07:43:35Z → 2026-07-26T09:17:12Z
- Build peak RSS: 3432.1 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.737 |
| Recall@5 (all) | 0.705 |
| Recall@10 (any) | 0.847 |
| Recall@10 (all) | 0.799 |
| MRR | 0.654 |
| FP-rate@5 | 0.277 |
| Correct-heading | 0.792 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.556 | 0.630 | 0.530 | n/a |
| heading | 24 | 0.917 | 0.958 | 0.722 | 0.792 |
| code-fragment | 14 | 0.786 | 0.857 | 0.620 | n/a |
| multi-note | 18 | 0.611 | 1.000 | 0.626 | n/a |
| exploratory | 17 | 0.588 | 0.706 | 0.502 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 0.944 | n/a |
| staleness | 7 | 0.571 | 0.571 | 0.362 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.706 | 0.824 | 0.615 | 0.895 |
| realistic | 33 | 0.818 | 0.909 | 0.754 | 0.400 |
| diagnostic | 7 | 0.571 | 0.571 | 0.362 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | — | no |
| C2 | which box handles name resolution at my parents' place | — | no |
| C3 | where does the long-running autonomous agent live | — | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 2 | no |
| C5 | where are the offsite copies of my git repositories kept | 6 | yes |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | — | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | — | no |
| C10 | the beans with the citrus and floral notes | 2 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 1 | yes |
| C12 | which of my data would actually hurt to lose | 1 | no |
| C13 | what should be switched off rather than kept running after I am gone | 1 | yes |
| C14 | something to get my dad | — | no |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | 1 | — |
| H5 | how are the media drives laid out | 1 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 1 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | 2 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 2 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 3 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 1 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 1 | — |
| F7 | 21116 udp forward | 1 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 2 | no |
| F10 | vm.swappiness=10 | 2 | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | yes |
| F12 | mnt-tmvol.mount | 7 | no |
| F13 | trivy-fleet-audit.timer | 5 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 3 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | no |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 1 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 7 | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 3 | yes |
| M6 | what should I use to build an interface that runs in the terminal | 1 | no |
| M7 | getting the car's charge level onto my dashboards | 7 | no |
| M8 | the wall shades we settled on and their fan-deck codes | 10 | no |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 7 | no |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | 7 | no |
| M14 | where do I keep track of which bags I have already bought | 8 | no |
| X1 | what should we do this weekend | 2 | no |
| X2 | something to put on tonight | — | no |
| X3 | I want to buy something nice for the flat | 1 | no |
| X4 | what am I meant to be reading | — | no |
| X5 | an idea I could actually sit down and build | — | no |
| X6 | keeping the machines patched and scanned for holes | 6 | no |
| X7 | how do I handle people I find hard to deal with | — | no |
| X8 | the general routine for looking after the indoor plants | 1 | yes |
| X9 | how am I going about picking up the language | 2 | no |
| X10 | what should I print next | — | no |
| X11 | ways to give an assistant a memory that persists | 1 | yes |
| X12 | how would I find out a disk is dying before it takes something with it | 4 | no |
| X13 | the rules for keeping these notes tidy | 1 | yes |
| X14 | the emotional side of becoming a father | 1 | no |
| X15 | something about accepting what you cannot change | 1 | no |
| X16 | how are we going to handle solids when the time comes | 9 | no |
| X17 | cheaper ways to rent compute | 1 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 3 | yes |
| H12 | what runs overnight, hour by hour | 1 | no |
| H13 | what happens if the key server is unreachable when a machine reboots | 6 | yes |
| H14 | which of the two feeds should I actually point the indexer at | — | no |
| H15 | how do I choose an ID when I create a new guest | 1 | yes |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 1 | no |
| H18 | which guest was actually filling up the backup store | 4 | yes |
| H19 | how do I get an off-site agent talking again after its key drifts | 1 | no |
| H20 | what has changed on the little N100 machine lately | 4 | yes |
| H21 | which packages did I deliberately tell the bot to leave alone | 2 | no |
| N1 | Kokuyo Campus notebook buying notes | 1 | no |
| C16 | which of the plants gets watered on a fixed weekly schedule | — | no |
| C17 | the one I should test with a finger instead of watering to a routine | — | no |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | — | no |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | 7 | no |
| C21 | how many kicks should I feel in two hours before ringing someone | 1 | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 1 | no |
| C24 | who do I call when a pipe leaks | 1 | yes |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | — | no |
| S2 | what does network-wide DNS filtering run on | — | no |
| S3 | how do I keep homelab secrets encrypted at rest | 5 | yes |
| S4 | how are container image updates automated | — | yes |
| S5 | how is the browser terminal exposed | 3 | no |
| S6 | the script that sets up my shell on a freshly built machine | 1 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 2 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 2 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 2 | — |
| U1 | Where does my Plex media live? | 1 | — |
| U2 | I'm looking for a new smell for the house | 1 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 7 | yes |
| U5 | I am travelling by plane with the baby | 1 | yes |
| U6 | How can I reflect on things with my family? | 1 | yes |

## EmbeddingGemma300MQ4 — chunk 800/50 · ctx on · dim native

- Run timestamp: 2026-07-26T09:33:06Z
- Build duration: 887.3 s
- Build window: 2026-07-26T09:18:11Z → 2026-07-26T09:32:58Z
- Build peak RSS: 547.3 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.915 |
| Recall@5 (all) | 0.856 |
| Recall@10 (any) | 0.958 |
| Recall@10 (all) | 0.929 |
| MRR | 0.801 |
| FP-rate@5 | 0.349 |
| Correct-heading | 0.792 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.889 | 0.963 | 0.753 | n/a |
| heading | 24 | 1.000 | 1.000 | 0.903 | 0.792 |
| code-fragment | 14 | 0.786 | 0.857 | 0.657 | n/a |
| multi-note | 18 | 0.889 | 0.944 | 0.726 | n/a |
| exploratory | 17 | 0.882 | 0.941 | 0.724 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 1.000 | n/a |
| staleness | 7 | 0.429 | 0.714 | 0.356 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.918 | 0.953 | 0.756 | 0.895 |
| realistic | 33 | 0.909 | 0.970 | 0.918 | 0.400 |
| diagnostic | 7 | 0.429 | 0.714 | 0.356 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | 1 | no |
| C2 | which box handles name resolution at my parents' place | 4 | no |
| C3 | where does the long-running autonomous agent live | 3 | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 3 | no |
| C5 | where are the offsite copies of my git repositories kept | 1 | no |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | 1 | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | 6 | yes |
| C10 | the beans with the citrus and floral notes | 4 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 1 | yes |
| C12 | which of my data would actually hurt to lose | 1 | yes |
| C13 | what should be switched off rather than kept running after I am gone | 1 | no |
| C14 | something to get my dad | 1 | yes |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | 1 | — |
| H5 | how are the media drives laid out | 2 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 1 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | 1 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 1 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 1 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 1 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 2 | — |
| F7 | 21116 udp forward | 3 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 1 | no |
| F10 | vm.swappiness=10 | 6 | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | 1 | no |
| F13 | trivy-fleet-audit.timer | 1 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 5 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | no |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 1 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 2 | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | 1 | no |
| M7 | getting the car's charge level onto my dashboards | 1 | yes |
| M8 | the wall shades we settled on and their fan-deck codes | — | no |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 7 | no |
| M12 | recipe books to give her | 1 | yes |
| M13 | the modular storage bin designs I bookmarked to print | 3 | no |
| M14 | where do I keep track of which bags I have already bought | 4 | no |
| X1 | what should we do this weekend | 1 | no |
| X2 | something to put on tonight | 1 | yes |
| X3 | I want to buy something nice for the flat | — | yes |
| X4 | what am I meant to be reading | 1 | no |
| X5 | an idea I could actually sit down and build | 3 | no |
| X6 | keeping the machines patched and scanned for holes | 3 | no |
| X7 | how do I handle people I find hard to deal with | 7 | yes |
| X8 | the general routine for looking after the indoor plants | 1 | yes |
| X9 | how am I going about picking up the language | 1 | no |
| X10 | what should I print next | 2 | yes |
| X11 | ways to give an assistant a memory that persists | 1 | yes |
| X12 | how would I find out a disk is dying before it takes something with it | 2 | no |
| X13 | the rules for keeping these notes tidy | 1 | yes |
| X14 | the emotional side of becoming a father | 1 | no |
| X15 | something about accepting what you cannot change | 1 | yes |
| X16 | how are we going to handle solids when the time comes | 2 | no |
| X17 | cheaper ways to rent compute | 1 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 1 | yes |
| H12 | what runs overnight, hour by hour | 1 | no |
| H13 | what happens if the key server is unreachable when a machine reboots | 3 | no |
| H14 | which of the two feeds should I actually point the indexer at | 1 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 1 | no |
| H18 | which guest was actually filling up the backup store | 1 | yes |
| H19 | how do I get an off-site agent talking again after its key drifts | 1 | no |
| H20 | what has changed on the little N100 machine lately | 2 | yes |
| H21 | which packages did I deliberately tell the bot to leave alone | 3 | no |
| N1 | Kokuyo Campus notebook buying notes | 1 | no |
| C16 | which of the plants gets watered on a fixed weekly schedule | 1 | yes |
| C17 | the one I should test with a finger instead of watering to a routine | 2 | no |
| C18 | where do we get Indonesian takeaway | 1 | no |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | 6 | yes |
| C21 | how many kicks should I feel in two hours before ringing someone | 3 | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 1 | no |
| C24 | who do I call when a pipe leaks | 1 | yes |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | 6 | yes |
| S2 | what does network-wide DNS filtering run on | — | yes |
| S3 | how do I keep homelab secrets encrypted at rest | — | yes |
| S4 | how are container image updates automated | 8 | yes |
| S5 | how is the browser terminal exposed | 5 | no |
| S6 | the script that sets up my shell on a freshly built machine | 1 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 1 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 2 | — |
| U2 | I'm looking for a new smell for the house | 3 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 1 | yes |
| U5 | I am travelling by plane with the baby | 1 | yes |
| U6 | How can I reflect on things with my family? | 1 | yes |

## EmbeddingGemma300MQ4 — chunk 800/50 · ctx on · dim 256

- Run timestamp: 2026-07-26T09:48:51Z
- Build duration: 935.3 s
- Build window: 2026-07-26T09:33:08Z → 2026-07-26T09:48:43Z
- Build peak RSS: 540.8 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.831 |
| Recall@5 (all) | 0.763 |
| Recall@10 (any) | 0.907 |
| Recall@10 (all) | 0.867 |
| MRR | 0.689 |
| FP-rate@5 | 0.349 |
| Correct-heading | 0.708 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.704 | 0.852 | 0.642 | n/a |
| heading | 24 | 0.917 | 1.000 | 0.748 | 0.708 |
| code-fragment | 14 | 0.643 | 0.786 | 0.506 | n/a |
| multi-note | 18 | 0.889 | 0.944 | 0.677 | n/a |
| exploratory | 17 | 0.824 | 0.824 | 0.544 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 0.972 | n/a |
| staleness | 7 | 0.286 | 0.571 | 0.333 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.835 | 0.918 | 0.657 | 0.789 |
| realistic | 33 | 0.818 | 0.879 | 0.772 | 0.400 |
| diagnostic | 7 | 0.286 | 0.571 | 0.333 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | 2 | no |
| C2 | which box handles name resolution at my parents' place | 6 | no |
| C3 | where does the long-running autonomous agent live | 9 | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 1 | no |
| C5 | where are the offsite copies of my git repositories kept | 1 | yes |
| C6 | what runs the retro gaming console plugged into the TV | 1 | yes |
| C7 | which server scans and files my paperwork | — | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | 8 | yes |
| C10 | the beans with the citrus and floral notes | — | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 1 | yes |
| C12 | which of my data would actually hurt to lose | 1 | yes |
| C13 | what should be switched off rather than kept running after I am gone | 1 | no |
| C14 | something to get my dad | 1 | no |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | 1 | — |
| H5 | how are the media drives laid out | 2 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 1 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | 2 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 10 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 9 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 2 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 6 | — |
| F7 | 21116 udp forward | 4 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 1 | no |
| F10 | vm.swappiness=10 | — | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | 6 | no |
| F13 | trivy-fleet-audit.timer | 2 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 2 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | no |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 1 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 2 | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 4 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | 1 | no |
| M7 | getting the car's charge level onto my dashboards | 1 | no |
| M8 | the wall shades we settled on and their fan-deck codes | — | no |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 10 | no |
| M12 | recipe books to give her | 1 | yes |
| M13 | the modular storage bin designs I bookmarked to print | 3 | no |
| M14 | where do I keep track of which bags I have already bought | 4 | no |
| X1 | what should we do this weekend | 1 | no |
| X2 | something to put on tonight | 2 | yes |
| X3 | I want to buy something nice for the flat | — | yes |
| X4 | what am I meant to be reading | — | no |
| X5 | an idea I could actually sit down and build | — | no |
| X6 | keeping the machines patched and scanned for holes | 2 | no |
| X7 | how do I handle people I find hard to deal with | 3 | yes |
| X8 | the general routine for looking after the indoor plants | 2 | yes |
| X9 | how am I going about picking up the language | 2 | no |
| X10 | what should I print next | 3 | yes |
| X11 | ways to give an assistant a memory that persists | 1 | no |
| X12 | how would I find out a disk is dying before it takes something with it | 3 | no |
| X13 | the rules for keeping these notes tidy | 1 | yes |
| X14 | the emotional side of becoming a father | 1 | no |
| X15 | something about accepting what you cannot change | 1 | yes |
| X16 | how are we going to handle solids when the time comes | 4 | no |
| X17 | cheaper ways to rent compute | 1 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 1 | no |
| H12 | what runs overnight, hour by hour | 1 | no |
| H13 | what happens if the key server is unreachable when a machine reboots | 1 | no |
| H14 | which of the two feeds should I actually point the indexer at | 3 | no |
| H15 | how do I choose an ID when I create a new guest | 5 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 1 | no |
| H18 | which guest was actually filling up the backup store | 2 | yes |
| H19 | how do I get an off-site agent talking again after its key drifts | 1 | no |
| H20 | what has changed on the little N100 machine lately | 1 | yes |
| H21 | which packages did I deliberately tell the bot to leave alone | 5 | no |
| N1 | Kokuyo Campus notebook buying notes | 1 | yes |
| C16 | which of the plants gets watered on a fixed weekly schedule | 2 | yes |
| C17 | the one I should test with a finger instead of watering to a routine | 1 | no |
| C18 | where do we get Indonesian takeaway | 1 | no |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | 9 | no |
| C21 | how many kicks should I feel in two hours before ringing someone | — | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | yes |
| C23 | that sweet with the crunchy shell and the jelly middle | 1 | yes |
| C24 | who do I call when a pipe leaks | 2 | yes |
| C25 | should she cut foods out of her diet if he is crying a lot | 3 | yes |
| S1 | how are the DAS shares mounted for the media stack | 6 | yes |
| S2 | what does network-wide DNS filtering run on | — | yes |
| S3 | how do I keep homelab secrets encrypted at rest | — | no |
| S4 | how are container image updates automated | 6 | yes |
| S5 | how is the browser terminal exposed | — | no |
| S6 | the script that sets up my shell on a freshly built machine | 1 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 2 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 1 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 2 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 1 | — |
| U2 | I'm looking for a new smell for the house | 4 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 2 | yes |
| U5 | I am travelling by plane with the baby | 1 | yes |
| U6 | How can I reflect on things with my family? | 1 | yes |

## SnowflakeArcticEmbedMV2 — chunk 800/50 · ctx on · dim native

- Run timestamp: 2026-07-26T10:06:59Z
- Build duration: 1071.3 s
- Build window: 2026-07-26T09:48:57Z → 2026-07-26T10:06:48Z
- Build peak RSS: 3119.4 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.907 |
| Recall@5 (all) | 0.855 |
| Recall@10 (any) | 0.932 |
| Recall@10 (all) | 0.898 |
| MRR | 0.810 |
| FP-rate@5 | 0.313 |
| Correct-heading | 0.792 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.852 | 0.852 | 0.710 | n/a |
| heading | 24 | 0.958 | 0.958 | 0.802 | 0.792 |
| code-fragment | 14 | 0.714 | 0.857 | 0.698 | n/a |
| multi-note | 18 | 1.000 | 1.000 | 0.889 | n/a |
| exploratory | 17 | 0.882 | 0.941 | 0.788 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 1.000 | n/a |
| staleness | 7 | 0.429 | 0.857 | 0.485 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.882 | 0.906 | 0.775 | 0.895 |
| realistic | 33 | 0.970 | 1.000 | 0.899 | 0.400 |
| diagnostic | 7 | 0.429 | 0.857 | 0.485 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | — | no |
| C2 | which box handles name resolution at my parents' place | 3 | no |
| C3 | where does the long-running autonomous agent live | 2 | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 2 | no |
| C5 | where are the offsite copies of my git repositories kept | 2 | yes |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | 3 | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | — | no |
| C10 | the beans with the citrus and floral notes | 1 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 2 | yes |
| C12 | which of my data would actually hurt to lose | 1 | no |
| C13 | what should be switched off rather than kept running after I am gone | 1 | yes |
| C14 | something to get my dad | 1 | yes |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | 1 | — |
| H5 | how are the media drives laid out | 1 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 1 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | 4 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 1 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 3 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 6 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 1 | — |
| F7 | 21116 udp forward | 1 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 1 | no |
| F10 | vm.swappiness=10 | 10 | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | 2 | no |
| F13 | trivy-fleet-audit.timer | 1 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 1 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | no |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 1 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 2 | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | 1 | no |
| M7 | getting the car's charge level onto my dashboards | 1 | no |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | no |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 1 | yes |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | 1 | no |
| M14 | where do I keep track of which bags I have already bought | 2 | no |
| X1 | what should we do this weekend | 1 | no |
| X2 | something to put on tonight | — | yes |
| X3 | I want to buy something nice for the flat | 1 | yes |
| X4 | what am I meant to be reading | 1 | yes |
| X5 | an idea I could actually sit down and build | 2 | yes |
| X6 | keeping the machines patched and scanned for holes | 1 | yes |
| X7 | how do I handle people I find hard to deal with | 2 | yes |
| X8 | the general routine for looking after the indoor plants | 1 | yes |
| X9 | how am I going about picking up the language | 7 | no |
| X10 | what should I print next | 1 | no |
| X11 | ways to give an assistant a memory that persists | 1 | yes |
| X12 | how would I find out a disk is dying before it takes something with it | 4 | no |
| X13 | the rules for keeping these notes tidy | 1 | no |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | no |
| X16 | how are we going to handle solids when the time comes | 1 | no |
| X17 | cheaper ways to rent compute | 1 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 1 | no |
| H12 | what runs overnight, hour by hour | 1 | no |
| H13 | what happens if the key server is unreachable when a machine reboots | 3 | no |
| H14 | which of the two feeds should I actually point the indexer at | 1 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 1 | no |
| H18 | which guest was actually filling up the backup store | 2 | yes |
| H19 | how do I get an off-site agent talking again after its key drifts | 1 | no |
| H20 | what has changed on the little N100 machine lately | 3 | no |
| H21 | which packages did I deliberately tell the bot to leave alone | — | no |
| N1 | Kokuyo Campus notebook buying notes | 1 | no |
| C16 | which of the plants gets watered on a fixed weekly schedule | 1 | no |
| C17 | the one I should test with a finger instead of watering to a routine | — | no |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | 1 | yes |
| C21 | how many kicks should I feel in two hours before ringing someone | 2 | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 1 | no |
| C24 | who do I call when a pipe leaks | 1 | no |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | 8 | yes |
| S2 | what does network-wide DNS filtering run on | — | no |
| S3 | how do I keep homelab secrets encrypted at rest | 8 | yes |
| S4 | how are container image updates automated | 7 | yes |
| S5 | how is the browser terminal exposed | 1 | no |
| S6 | the script that sets up my shell on a freshly built machine | 1 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 2 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 1 | — |
| U2 | I'm looking for a new smell for the house | 2 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 2 | yes |
| U5 | I am travelling by plane with the baby | 1 | yes |
| U6 | How can I reflect on things with my family? | 1 | yes |

## SnowflakeArcticEmbedMV2 — chunk 800/50 · ctx on · dim 256

- Run timestamp: 2026-07-26T10:24:53Z
- Build duration: 1059.2 s
- Build window: 2026-07-26T10:07:03Z → 2026-07-26T10:24:42Z
- Build peak RSS: 3119.0 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.898 |
| Recall@5 (all) | 0.833 |
| Recall@10 (any) | 0.915 |
| Recall@10 (all) | 0.874 |
| MRR | 0.780 |
| FP-rate@5 | 0.325 |
| Correct-heading | 0.792 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.815 | 0.852 | 0.684 | n/a |
| heading | 24 | 0.917 | 0.958 | 0.732 | 0.792 |
| code-fragment | 14 | 0.714 | 0.714 | 0.679 | n/a |
| multi-note | 18 | 1.000 | 1.000 | 0.889 | n/a |
| exploratory | 17 | 0.941 | 0.941 | 0.734 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 1.000 | n/a |
| staleness | 7 | 0.429 | 0.857 | 0.480 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.871 | 0.894 | 0.734 | 0.895 |
| realistic | 33 | 0.970 | 0.970 | 0.896 | 0.400 |
| diagnostic | 7 | 0.429 | 0.857 | 0.480 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | — | no |
| C2 | which box handles name resolution at my parents' place | 4 | no |
| C3 | where does the long-running autonomous agent live | 3 | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 2 | no |
| C5 | where are the offsite copies of my git repositories kept | 8 | no |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | 4 | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | — | no |
| C10 | the beans with the citrus and floral notes | 1 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 2 | yes |
| C12 | which of my data would actually hurt to lose | 1 | no |
| C13 | what should be switched off rather than kept running after I am gone | 1 | yes |
| C14 | something to get my dad | 1 | yes |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | 1 | — |
| H5 | how are the media drives laid out | 1 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 1 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | 3 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 1 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 4 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | — | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 1 | — |
| F7 | 21116 udp forward | 1 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 1 | no |
| F10 | vm.swappiness=10 | — | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | 2 | no |
| F13 | trivy-fleet-audit.timer | 1 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 1 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | no |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 1 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 2 | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | 1 | no |
| M7 | getting the car's charge level onto my dashboards | 1 | no |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | no |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 1 | yes |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | 1 | no |
| M14 | where do I keep track of which bags I have already bought | 2 | no |
| X1 | what should we do this weekend | 1 | no |
| X2 | something to put on tonight | — | yes |
| X3 | I want to buy something nice for the flat | 1 | yes |
| X4 | what am I meant to be reading | 1 | yes |
| X5 | an idea I could actually sit down and build | 4 | no |
| X6 | keeping the machines patched and scanned for holes | 1 | yes |
| X7 | how do I handle people I find hard to deal with | 3 | yes |
| X8 | the general routine for looking after the indoor plants | 1 | yes |
| X9 | how am I going about picking up the language | 5 | no |
| X10 | what should I print next | 1 | no |
| X11 | ways to give an assistant a memory that persists | 1 | yes |
| X12 | how would I find out a disk is dying before it takes something with it | 5 | no |
| X13 | the rules for keeping these notes tidy | 2 | no |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | yes |
| X16 | how are we going to handle solids when the time comes | 1 | no |
| X17 | cheaper ways to rent compute | 1 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 1 | no |
| H12 | what runs overnight, hour by hour | 1 | no |
| H13 | what happens if the key server is unreachable when a machine reboots | 3 | no |
| H14 | which of the two feeds should I actually point the indexer at | 3 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 1 | no |
| H18 | which guest was actually filling up the backup store | 3 | yes |
| H19 | how do I get an off-site agent talking again after its key drifts | 3 | no |
| H20 | what has changed on the little N100 machine lately | 7 | no |
| H21 | which packages did I deliberately tell the bot to leave alone | — | no |
| N1 | Kokuyo Campus notebook buying notes | 1 | no |
| C16 | which of the plants gets watered on a fixed weekly schedule | 1 | yes |
| C17 | the one I should test with a finger instead of watering to a routine | — | no |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | 1 | yes |
| C21 | how many kicks should I feel in two hours before ringing someone | 2 | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 1 | no |
| C24 | who do I call when a pipe leaks | 1 | yes |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | 8 | yes |
| S2 | what does network-wide DNS filtering run on | — | no |
| S3 | how do I keep homelab secrets encrypted at rest | 8 | yes |
| S4 | how are container image updates automated | 9 | yes |
| S5 | how is the browser terminal exposed | 1 | no |
| S6 | the script that sets up my shell on a freshly built machine | 1 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 2 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 1 | — |
| U2 | I'm looking for a new smell for the house | 2 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 2 | yes |
| U5 | I am travelling by plane with the baby | 1 | yes |
| U6 | How can I reflect on things with my family? | 1 | yes |

## EmbeddingGemma300MQ4 · retrieval-format v1 — chunk 800/50 · ctx on · dim native

- Run timestamp: 2026-07-26T11:44:42Z
- Build duration: 1032.9 s
- Build window: 2026-07-26T11:27:17Z → 2026-07-26T11:44:30Z
- Build peak RSS: 537.4 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.958 |
| Recall@5 (all) | 0.895 |
| Recall@10 (any) | 0.958 |
| Recall@10 (all) | 0.930 |
| MRR | 0.846 |
| FP-rate@5 | 0.361 |
| Correct-heading | 0.833 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.889 | 0.889 | 0.727 | n/a |
| heading | 24 | 1.000 | 1.000 | 0.892 | 0.833 |
| code-fragment | 14 | 0.857 | 0.857 | 0.821 | n/a |
| multi-note | 18 | 1.000 | 1.000 | 0.866 | n/a |
| exploratory | 17 | 1.000 | 1.000 | 0.806 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 1.000 | n/a |
| staleness | 7 | 0.571 | 1.000 | 0.568 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.941 | 0.941 | 0.795 | 0.947 |
| realistic | 33 | 1.000 | 1.000 | 0.977 | 0.400 |
| diagnostic | 7 | 0.571 | 1.000 | 0.568 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | 1 | no |
| C2 | which box handles name resolution at my parents' place | — | no |
| C3 | where does the long-running autonomous agent live | 2 | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 4 | no |
| C5 | where are the offsite copies of my git repositories kept | 5 | no |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | 1 | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | 3 | no |
| C10 | the beans with the citrus and floral notes | 2 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 1 | yes |
| C12 | which of my data would actually hurt to lose | 1 | no |
| C13 | what should be switched off rather than kept running after I am gone | 1 | no |
| C14 | something to get my dad | 1 | yes |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | 1 | — |
| H5 | how are the media drives laid out | 1 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 1 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | 1 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 1 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 1 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 1 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 2 | — |
| F7 | 21116 udp forward | 1 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 1 | no |
| F10 | vm.swappiness=10 | 1 | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | 1 | no |
| F13 | trivy-fleet-audit.timer | 1 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 1 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | no |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 1 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 2 | yes |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | 1 | no |
| M7 | getting the car's charge level onto my dashboards | 1 | yes |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | no |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 4 | no |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | 1 | no |
| M14 | where do I keep track of which bags I have already bought | 3 | no |
| X1 | what should we do this weekend | 1 | yes |
| X2 | something to put on tonight | 1 | yes |
| X3 | I want to buy something nice for the flat | 2 | yes |
| X4 | what am I meant to be reading | 1 | no |
| X5 | an idea I could actually sit down and build | 4 | no |
| X6 | keeping the machines patched and scanned for holes | 1 | yes |
| X7 | how do I handle people I find hard to deal with | 1 | yes |
| X8 | the general routine for looking after the indoor plants | 5 | yes |
| X9 | how am I going about picking up the language | 1 | no |
| X10 | what should I print next | 1 | no |
| X11 | ways to give an assistant a memory that persists | 2 | no |
| X12 | how would I find out a disk is dying before it takes something with it | 4 | no |
| X13 | the rules for keeping these notes tidy | 1 | yes |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | no |
| X16 | how are we going to handle solids when the time comes | 1 | no |
| X17 | cheaper ways to rent compute | 1 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 1 | yes |
| H12 | what runs overnight, hour by hour | 1 | yes |
| H13 | what happens if the key server is unreachable when a machine reboots | 3 | yes |
| H14 | which of the two feeds should I actually point the indexer at | 1 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 1 | no |
| H18 | which guest was actually filling up the backup store | 3 | yes |
| H19 | how do I get an off-site agent talking again after its key drifts | 1 | no |
| H20 | what has changed on the little N100 machine lately | 4 | no |
| H21 | which packages did I deliberately tell the bot to leave alone | 2 | no |
| N1 | Kokuyo Campus notebook buying notes | 1 | yes |
| C16 | which of the plants gets watered on a fixed weekly schedule | 1 | yes |
| C17 | the one I should test with a finger instead of watering to a routine | — | no |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | 3 | no |
| C21 | how many kicks should I feel in two hours before ringing someone | 2 | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 1 | yes |
| C24 | who do I call when a pipe leaks | 1 | yes |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | 6 | yes |
| S2 | what does network-wide DNS filtering run on | 6 | no |
| S3 | how do I keep homelab secrets encrypted at rest | 2 | yes |
| S4 | how are container image updates automated | 7 | yes |
| S5 | how is the browser terminal exposed | 1 | no |
| S6 | the script that sets up my shell on a freshly built machine | 1 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 1 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 1 | — |
| U2 | I'm looking for a new smell for the house | 2 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 1 | yes |
| U5 | I am travelling by plane with the baby | 1 | yes |
| U6 | How can I reflect on things with my family? | 1 | yes |

## EmbeddingGemma300MQ4 · retrieval-format v1 — chunk 450/50 · ctx on · dim native

- Run timestamp: 2026-07-26T12:02:10Z
- Build duration: 1033.6 s
- Build window: 2026-07-26T11:44:46Z → 2026-07-26T12:01:59Z
- Build peak RSS: 537.7 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.941 |
| Recall@5 (all) | 0.875 |
| Recall@10 (any) | 0.958 |
| Recall@10 (all) | 0.920 |
| MRR | 0.830 |
| FP-rate@5 | 0.337 |
| Correct-heading | 0.958 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.852 | 0.889 | 0.737 | n/a |
| heading | 24 | 1.000 | 1.000 | 0.843 | 0.958 |
| code-fragment | 14 | 0.857 | 0.857 | 0.810 | n/a |
| multi-note | 18 | 1.000 | 1.000 | 0.854 | n/a |
| exploratory | 17 | 0.941 | 1.000 | 0.772 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 1.000 | n/a |
| staleness | 7 | 0.714 | 0.857 | 0.521 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.918 | 0.941 | 0.781 | 0.947 |
| realistic | 33 | 1.000 | 1.000 | 0.956 | 1.000 |
| diagnostic | 7 | 0.714 | 0.857 | 0.521 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | 1 | no |
| C2 | which box handles name resolution at my parents' place | — | no |
| C3 | where does the long-running autonomous agent live | 1 | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 5 | no |
| C5 | where are the offsite copies of my git repositories kept | 9 | yes |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | 1 | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | 3 | no |
| C10 | the beans with the citrus and floral notes | 2 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 1 | yes |
| C12 | which of my data would actually hurt to lose | 1 | no |
| C13 | what should be switched off rather than kept running after I am gone | 1 | no |
| C14 | something to get my dad | 1 | no |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | 1 | — |
| H5 | how are the media drives laid out | 1 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 2 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | 1 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 1 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 1 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 1 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 1 | — |
| F7 | 21116 udp forward | 1 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 1 | no |
| F10 | vm.swappiness=10 | 1 | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | 3 | no |
| F13 | trivy-fleet-audit.timer | 1 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 1 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | no |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 1 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 1 | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | 1 | no |
| M7 | getting the car's charge level onto my dashboards | 1 | no |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | no |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 5 | no |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | 1 | no |
| M14 | where do I keep track of which bags I have already bought | 3 | no |
| X1 | what should we do this weekend | 1 | yes |
| X2 | something to put on tonight | 1 | yes |
| X3 | I want to buy something nice for the flat | 2 | yes |
| X4 | what am I meant to be reading | 1 | no |
| X5 | an idea I could actually sit down and build | 4 | no |
| X6 | keeping the machines patched and scanned for holes | 2 | yes |
| X7 | how do I handle people I find hard to deal with | 1 | yes |
| X8 | the general routine for looking after the indoor plants | 5 | yes |
| X9 | how am I going about picking up the language | 1 | no |
| X10 | what should I print next | 1 | no |
| X11 | ways to give an assistant a memory that persists | 2 | no |
| X12 | how would I find out a disk is dying before it takes something with it | 6 | no |
| X13 | the rules for keeping these notes tidy | 1 | yes |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | no |
| X16 | how are we going to handle solids when the time comes | 1 | no |
| X17 | cheaper ways to rent compute | 1 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 1 | yes |
| H12 | what runs overnight, hour by hour | 2 | yes |
| H13 | what happens if the key server is unreachable when a machine reboots | 5 | no |
| H14 | which of the two feeds should I actually point the indexer at | 1 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 1 | no |
| H18 | which guest was actually filling up the backup store | 2 | yes |
| H19 | how do I get an off-site agent talking again after its key drifts | 1 | no |
| H20 | what has changed on the little N100 machine lately | 5 | no |
| H21 | which packages did I deliberately tell the bot to leave alone | 3 | yes |
| N1 | Kokuyo Campus notebook buying notes | 1 | yes |
| C16 | which of the plants gets watered on a fixed weekly schedule | 1 | yes |
| C17 | the one I should test with a finger instead of watering to a routine | — | no |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | 4 | no |
| C21 | how many kicks should I feel in two hours before ringing someone | 2 | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 1 | yes |
| C24 | who do I call when a pipe leaks | 1 | yes |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | 5 | yes |
| S2 | what does network-wide DNS filtering run on | — | yes |
| S3 | how do I keep homelab secrets encrypted at rest | 3 | yes |
| S4 | how are container image updates automated | 9 | yes |
| S5 | how is the browser terminal exposed | 1 | no |
| S6 | the script that sets up my shell on a freshly built machine | 1 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 1 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 1 | — |
| U2 | I'm looking for a new smell for the house | 2 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 3 | yes |
| U5 | I am travelling by plane with the baby | 1 | yes |
| U6 | How can I reflect on things with my family? | 1 | yes |

## EmbeddingGemma300MQ4 · retrieval-format v1 — chunk 1200/75 · ctx on · dim native

- Run timestamp: 2026-07-26T12:18:30Z
- Build duration: 968.7 s
- Build window: 2026-07-26T12:02:13Z → 2026-07-26T12:18:21Z
- Build peak RSS: 538.6 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.932 |
| Recall@5 (all) | 0.885 |
| Recall@10 (any) | 0.966 |
| Recall@10 (all) | 0.934 |
| MRR | 0.835 |
| FP-rate@5 | 0.361 |
| Correct-heading | 0.500 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.852 | 0.963 | 0.749 | n/a |
| heading | 24 | 0.958 | 0.958 | 0.851 | 0.500 |
| code-fragment | 14 | 0.857 | 0.857 | 0.743 | n/a |
| multi-note | 18 | 0.944 | 1.000 | 0.861 | n/a |
| exploratory | 17 | 1.000 | 1.000 | 0.821 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 1.000 | n/a |
| staleness | 7 | 0.714 | 1.000 | 0.574 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.918 | 0.953 | 0.799 | 0.526 |
| realistic | 33 | 0.970 | 1.000 | 0.928 | 0.400 |
| diagnostic | 7 | 0.714 | 1.000 | 0.574 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | 1 | no |
| C2 | which box handles name resolution at my parents' place | 10 | no |
| C3 | where does the long-running autonomous agent live | 1 | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 3 | no |
| C5 | where are the offsite copies of my git repositories kept | 4 | no |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | 1 | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | 3 | no |
| C10 | the beans with the citrus and floral notes | 2 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 1 | yes |
| C12 | which of my data would actually hurt to lose | 1 | no |
| C13 | what should be switched off rather than kept running after I am gone | 1 | no |
| C14 | something to get my dad | 1 | yes |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | 1 | — |
| H5 | how are the media drives laid out | 1 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 1 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | 4 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 1 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 1 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 1 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 1 | — |
| F7 | 21116 udp forward | 1 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 1 | no |
| F10 | vm.swappiness=10 | 5 | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | 1 | no |
| F13 | trivy-fleet-audit.timer | 1 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 5 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | no |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 1 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 1 | no |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | 1 | no |
| M7 | getting the car's charge level onto my dashboards | 1 | yes |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | no |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 6 | no |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | 1 | no |
| M14 | where do I keep track of which bags I have already bought | 3 | no |
| X1 | what should we do this weekend | 1 | yes |
| X2 | something to put on tonight | 1 | yes |
| X3 | I want to buy something nice for the flat | 2 | yes |
| X4 | what am I meant to be reading | 1 | no |
| X5 | an idea I could actually sit down and build | 4 | no |
| X6 | keeping the machines patched and scanned for holes | 1 | yes |
| X7 | how do I handle people I find hard to deal with | 1 | yes |
| X8 | the general routine for looking after the indoor plants | 5 | yes |
| X9 | how am I going about picking up the language | 1 | no |
| X10 | what should I print next | 1 | no |
| X11 | ways to give an assistant a memory that persists | 2 | no |
| X12 | how would I find out a disk is dying before it takes something with it | 2 | no |
| X13 | the rules for keeping these notes tidy | 1 | yes |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | no |
| X16 | how are we going to handle solids when the time comes | 1 | no |
| X17 | cheaper ways to rent compute | 1 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 1 | yes |
| H12 | what runs overnight, hour by hour | 1 | yes |
| H13 | what happens if the key server is unreachable when a machine reboots | 3 | yes |
| H14 | which of the two feeds should I actually point the indexer at | — | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 1 | no |
| H18 | which guest was actually filling up the backup store | 1 | no |
| H19 | how do I get an off-site agent talking again after its key drifts | 1 | no |
| H20 | what has changed on the little N100 machine lately | 3 | no |
| H21 | which packages did I deliberately tell the bot to leave alone | 2 | yes |
| N1 | Kokuyo Campus notebook buying notes | 1 | yes |
| C16 | which of the plants gets watered on a fixed weekly schedule | 1 | yes |
| C17 | the one I should test with a finger instead of watering to a routine | 9 | no |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | 10 | yes |
| C21 | how many kicks should I feel in two hours before ringing someone | 2 | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 1 | yes |
| C24 | who do I call when a pipe leaks | 1 | yes |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | 7 | yes |
| S2 | what does network-wide DNS filtering run on | 4 | no |
| S3 | how do I keep homelab secrets encrypted at rest | 2 | yes |
| S4 | how are container image updates automated | 8 | yes |
| S5 | how is the browser terminal exposed | 1 | no |
| S6 | the script that sets up my shell on a freshly built machine | 1 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 1 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 1 | — |
| U2 | I'm looking for a new smell for the house | 2 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 2 | yes |
| U5 | I am travelling by plane with the baby | 1 | yes |
| U6 | How can I reflect on things with my family? | 1 | yes |

## EmbeddingGemma300MQ4 · retrieval-format v1 — chunk 1600/100 · ctx on · dim native

- Run timestamp: 2026-07-26T12:37:32Z
- Build duration: 1130.6 s
- Build window: 2026-07-26T12:18:33Z → 2026-07-26T12:37:23Z
- Build peak RSS: 589.6 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.924 |
| Recall@5 (all) | 0.872 |
| Recall@10 (any) | 0.958 |
| Recall@10 (all) | 0.926 |
| MRR | 0.801 |
| FP-rate@5 | 0.422 |
| Correct-heading | 0.083 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.852 | 0.926 | 0.700 | n/a |
| heading | 24 | 0.875 | 0.958 | 0.790 | 0.083 |
| code-fragment | 14 | 0.857 | 0.857 | 0.738 | n/a |
| multi-note | 18 | 1.000 | 1.000 | 0.835 | n/a |
| exploratory | 17 | 1.000 | 1.000 | 0.821 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 0.963 | n/a |
| staleness | 7 | 0.857 | 1.000 | 0.578 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.894 | 0.941 | 0.762 | 0.053 |
| realistic | 33 | 1.000 | 1.000 | 0.903 | 0.200 |
| diagnostic | 7 | 0.857 | 1.000 | 0.578 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | 1 | no |
| C2 | which box handles name resolution at my parents' place | 8 | no |
| C3 | where does the long-running autonomous agent live | 4 | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 3 | no |
| C5 | where are the offsite copies of my git repositories kept | — | yes |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | 2 | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | 3 | no |
| C10 | the beans with the citrus and floral notes | 2 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 1 | yes |
| C12 | which of my data would actually hurt to lose | 1 | no |
| C13 | what should be switched off rather than kept running after I am gone | 1 | yes |
| C14 | something to get my dad | 1 | yes |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | 1 | — |
| H5 | how are the media drives laid out | 7 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 5 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | 4 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 1 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 1 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 1 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 3 | — |
| F7 | 21116 udp forward | 1 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 1 | no |
| F10 | vm.swappiness=10 | 2 | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | 1 | no |
| F13 | trivy-fleet-audit.timer | 1 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 2 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | no |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 1 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 1 | yes |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | 1 | no |
| M7 | getting the car's charge level onto my dashboards | 1 | yes |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | no |
| M9 | where are the scan images from the pregnancy | 1 | yes |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 5 | no |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | 1 | no |
| M14 | where do I keep track of which bags I have already bought | 3 | no |
| X1 | what should we do this weekend | 1 | yes |
| X2 | something to put on tonight | 1 | yes |
| X3 | I want to buy something nice for the flat | 2 | yes |
| X4 | what am I meant to be reading | 1 | no |
| X5 | an idea I could actually sit down and build | 4 | no |
| X6 | keeping the machines patched and scanned for holes | 1 | yes |
| X7 | how do I handle people I find hard to deal with | 1 | yes |
| X8 | the general routine for looking after the indoor plants | 5 | yes |
| X9 | how am I going about picking up the language | 1 | no |
| X10 | what should I print next | 1 | no |
| X11 | ways to give an assistant a memory that persists | 2 | no |
| X12 | how would I find out a disk is dying before it takes something with it | 2 | no |
| X13 | the rules for keeping these notes tidy | 1 | yes |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | no |
| X16 | how are we going to handle solids when the time comes | 1 | no |
| X17 | cheaper ways to rent compute | 1 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 1 | yes |
| H12 | what runs overnight, hour by hour | 1 | yes |
| H13 | what happens if the key server is unreachable when a machine reboots | 6 | yes |
| H14 | which of the two feeds should I actually point the indexer at | — | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 1 | no |
| H18 | which guest was actually filling up the backup store | 1 | yes |
| H19 | how do I get an off-site agent talking again after its key drifts | 1 | no |
| H20 | what has changed on the little N100 machine lately | 5 | yes |
| H21 | which packages did I deliberately tell the bot to leave alone | 1 | yes |
| N1 | Kokuyo Campus notebook buying notes | 1 | yes |
| C16 | which of the plants gets watered on a fixed weekly schedule | 1 | yes |
| C17 | the one I should test with a finger instead of watering to a routine | 5 | no |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | 6 | no |
| C21 | how many kicks should I feel in two hours before ringing someone | 2 | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 1 | yes |
| C24 | who do I call when a pipe leaks | 1 | yes |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | 5 | yes |
| S2 | what does network-wide DNS filtering run on | 7 | no |
| S3 | how do I keep homelab secrets encrypted at rest | 2 | yes |
| S4 | how are container image updates automated | 5 | yes |
| S5 | how is the browser terminal exposed | 1 | no |
| S6 | the script that sets up my shell on a freshly built machine | 1 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 1 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 3 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 2 | — |
| U2 | I'm looking for a new smell for the house | 2 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 2 | yes |
| U5 | I am travelling by plane with the baby | 1 | yes |
| U6 | How can I reflect on things with my family? | 1 | yes |

## EmbeddingGemma300MQ4 · retrieval-format v1 — chunk 800/0 · ctx on · dim native

- Run timestamp: 2026-07-26T12:53:03Z
- Build duration: 918.0 s
- Build window: 2026-07-26T12:37:35Z → 2026-07-26T12:52:53Z
- Build peak RSS: 537.7 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.958 |
| Recall@5 (all) | 0.898 |
| Recall@10 (any) | 0.958 |
| Recall@10 (all) | 0.930 |
| MRR | 0.846 |
| FP-rate@5 | 0.361 |
| Correct-heading | 0.833 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.889 | 0.889 | 0.727 | n/a |
| heading | 24 | 1.000 | 1.000 | 0.892 | 0.833 |
| code-fragment | 14 | 0.857 | 0.857 | 0.821 | n/a |
| multi-note | 18 | 1.000 | 1.000 | 0.866 | n/a |
| exploratory | 17 | 1.000 | 1.000 | 0.806 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 1.000 | n/a |
| staleness | 7 | 0.571 | 1.000 | 0.568 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.941 | 0.941 | 0.795 | 0.947 |
| realistic | 33 | 1.000 | 1.000 | 0.977 | 0.400 |
| diagnostic | 7 | 0.571 | 1.000 | 0.568 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | 1 | no |
| C2 | which box handles name resolution at my parents' place | — | no |
| C3 | where does the long-running autonomous agent live | 2 | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 4 | no |
| C5 | where are the offsite copies of my git repositories kept | 5 | no |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | 1 | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | 3 | no |
| C10 | the beans with the citrus and floral notes | 2 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 1 | yes |
| C12 | which of my data would actually hurt to lose | 1 | no |
| C13 | what should be switched off rather than kept running after I am gone | 1 | no |
| C14 | something to get my dad | 1 | yes |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | 1 | — |
| H5 | how are the media drives laid out | 1 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 1 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | 1 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 1 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 1 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 1 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 2 | — |
| F7 | 21116 udp forward | 1 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 1 | no |
| F10 | vm.swappiness=10 | 1 | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | 1 | no |
| F13 | trivy-fleet-audit.timer | 1 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 1 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | no |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 1 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 2 | yes |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | 1 | no |
| M7 | getting the car's charge level onto my dashboards | 1 | yes |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | no |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 4 | no |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | 1 | no |
| M14 | where do I keep track of which bags I have already bought | 3 | no |
| X1 | what should we do this weekend | 1 | yes |
| X2 | something to put on tonight | 1 | yes |
| X3 | I want to buy something nice for the flat | 2 | yes |
| X4 | what am I meant to be reading | 1 | no |
| X5 | an idea I could actually sit down and build | 4 | no |
| X6 | keeping the machines patched and scanned for holes | 1 | yes |
| X7 | how do I handle people I find hard to deal with | 1 | yes |
| X8 | the general routine for looking after the indoor plants | 5 | yes |
| X9 | how am I going about picking up the language | 1 | no |
| X10 | what should I print next | 1 | no |
| X11 | ways to give an assistant a memory that persists | 2 | no |
| X12 | how would I find out a disk is dying before it takes something with it | 4 | no |
| X13 | the rules for keeping these notes tidy | 1 | yes |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | no |
| X16 | how are we going to handle solids when the time comes | 1 | no |
| X17 | cheaper ways to rent compute | 1 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 1 | yes |
| H12 | what runs overnight, hour by hour | 1 | yes |
| H13 | what happens if the key server is unreachable when a machine reboots | 3 | yes |
| H14 | which of the two feeds should I actually point the indexer at | 1 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 1 | no |
| H18 | which guest was actually filling up the backup store | 3 | yes |
| H19 | how do I get an off-site agent talking again after its key drifts | 1 | no |
| H20 | what has changed on the little N100 machine lately | 4 | no |
| H21 | which packages did I deliberately tell the bot to leave alone | 2 | no |
| N1 | Kokuyo Campus notebook buying notes | 1 | yes |
| C16 | which of the plants gets watered on a fixed weekly schedule | 1 | yes |
| C17 | the one I should test with a finger instead of watering to a routine | — | no |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | 3 | no |
| C21 | how many kicks should I feel in two hours before ringing someone | 2 | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 1 | yes |
| C24 | who do I call when a pipe leaks | 1 | yes |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | 6 | yes |
| S2 | what does network-wide DNS filtering run on | 6 | no |
| S3 | how do I keep homelab secrets encrypted at rest | 2 | yes |
| S4 | how are container image updates automated | 7 | yes |
| S5 | how is the browser terminal exposed | 1 | no |
| S6 | the script that sets up my shell on a freshly built machine | 1 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 1 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 1 | — |
| U2 | I'm looking for a new smell for the house | 2 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 1 | yes |
| U5 | I am travelling by plane with the baby | 1 | yes |
| U6 | How can I reflect on things with my family? | 1 | yes |

## EmbeddingGemma300MQ4 · retrieval-format v1 — chunk 800/100 · ctx on · dim native

- Run timestamp: 2026-07-26T13:09:15Z
- Build duration: 960.9 s
- Build window: 2026-07-26T12:53:05Z → 2026-07-26T13:09:06Z
- Build peak RSS: 537.8 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.958 |
| Recall@5 (all) | 0.900 |
| Recall@10 (any) | 0.958 |
| Recall@10 (all) | 0.930 |
| MRR | 0.838 |
| FP-rate@5 | 0.361 |
| Correct-heading | 0.833 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.889 | 0.889 | 0.727 | n/a |
| heading | 24 | 1.000 | 1.000 | 0.875 | 0.833 |
| code-fragment | 14 | 0.857 | 0.857 | 0.786 | n/a |
| multi-note | 18 | 1.000 | 1.000 | 0.866 | n/a |
| exploratory | 17 | 1.000 | 1.000 | 0.806 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 1.000 | n/a |
| staleness | 7 | 0.571 | 1.000 | 0.568 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.941 | 0.941 | 0.790 | 0.947 |
| realistic | 33 | 1.000 | 1.000 | 0.962 | 0.400 |
| diagnostic | 7 | 0.571 | 1.000 | 0.568 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | 1 | no |
| C2 | which box handles name resolution at my parents' place | — | no |
| C3 | where does the long-running autonomous agent live | 2 | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 4 | no |
| C5 | where are the offsite copies of my git repositories kept | 5 | no |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | 1 | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | 3 | no |
| C10 | the beans with the citrus and floral notes | 2 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 1 | yes |
| C12 | which of my data would actually hurt to lose | 1 | no |
| C13 | what should be switched off rather than kept running after I am gone | 1 | no |
| C14 | something to get my dad | 1 | yes |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | 1 | — |
| H5 | how are the media drives laid out | 1 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 2 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | 1 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 1 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 1 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 1 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 2 | — |
| F7 | 21116 udp forward | 1 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 1 | no |
| F10 | vm.swappiness=10 | 1 | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | 2 | no |
| F13 | trivy-fleet-audit.timer | 1 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 1 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | no |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 1 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 2 | yes |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | 1 | no |
| M7 | getting the car's charge level onto my dashboards | 1 | yes |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | no |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 4 | no |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | 1 | no |
| M14 | where do I keep track of which bags I have already bought | 3 | no |
| X1 | what should we do this weekend | 1 | yes |
| X2 | something to put on tonight | 1 | yes |
| X3 | I want to buy something nice for the flat | 2 | yes |
| X4 | what am I meant to be reading | 1 | no |
| X5 | an idea I could actually sit down and build | 4 | no |
| X6 | keeping the machines patched and scanned for holes | 1 | yes |
| X7 | how do I handle people I find hard to deal with | 1 | yes |
| X8 | the general routine for looking after the indoor plants | 5 | yes |
| X9 | how am I going about picking up the language | 1 | no |
| X10 | what should I print next | 1 | no |
| X11 | ways to give an assistant a memory that persists | 2 | no |
| X12 | how would I find out a disk is dying before it takes something with it | 4 | no |
| X13 | the rules for keeping these notes tidy | 1 | yes |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | no |
| X16 | how are we going to handle solids when the time comes | 1 | no |
| X17 | cheaper ways to rent compute | 1 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 1 | yes |
| H12 | what runs overnight, hour by hour | 1 | yes |
| H13 | what happens if the key server is unreachable when a machine reboots | 3 | yes |
| H14 | which of the two feeds should I actually point the indexer at | 1 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 1 | no |
| H18 | which guest was actually filling up the backup store | 3 | yes |
| H19 | how do I get an off-site agent talking again after its key drifts | 1 | no |
| H20 | what has changed on the little N100 machine lately | 3 | no |
| H21 | which packages did I deliberately tell the bot to leave alone | 2 | no |
| N1 | Kokuyo Campus notebook buying notes | 1 | yes |
| C16 | which of the plants gets watered on a fixed weekly schedule | 1 | yes |
| C17 | the one I should test with a finger instead of watering to a routine | — | no |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | 3 | no |
| C21 | how many kicks should I feel in two hours before ringing someone | 2 | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 1 | yes |
| C24 | who do I call when a pipe leaks | 1 | yes |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | 6 | yes |
| S2 | what does network-wide DNS filtering run on | 6 | no |
| S3 | how do I keep homelab secrets encrypted at rest | 2 | yes |
| S4 | how are container image updates automated | 7 | yes |
| S5 | how is the browser terminal exposed | 1 | no |
| S6 | the script that sets up my shell on a freshly built machine | 1 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 1 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 1 | — |
| U2 | I'm looking for a new smell for the house | 2 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 1 | yes |
| U5 | I am travelling by plane with the baby | 1 | yes |
| U6 | How can I reflect on things with my family? | 1 | yes |

## EmbeddingGemma300MQ4 · retrieval-format v1 — chunk 800/0 · ctx on · dim native · batch 1

- Run timestamp: 2026-07-26T15:16:25Z
- Build duration: 918.0 s
- Build window: 2026-07-26T12:37:35Z → 2026-07-26T12:52:53Z
- Build peak RSS: 537.7 MB

| Metric | Value |
|---|---|
| Recall@5 (any) | 0.958 |
| Recall@5 (all) | 0.898 |
| Recall@10 (any) | 0.958 |
| Recall@10 (all) | 0.930 |
| MRR | 0.846 |
| FP-rate@5 | 0.361 |
| Correct-heading | 0.833 |

### Per-category

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| conceptual | 27 | 0.889 | 0.889 | 0.727 | n/a |
| heading | 24 | 1.000 | 1.000 | 0.892 | 0.833 |
| code-fragment | 14 | 0.857 | 0.857 | 0.821 | n/a |
| multi-note | 18 | 1.000 | 1.000 | 0.866 | n/a |
| exploratory | 17 | 1.000 | 1.000 | 0.806 | n/a |
| exact-name | 18 | 1.000 | 1.000 | 1.000 | n/a |
| staleness | 7 | 0.571 | 1.000 | 0.568 | n/a |

### Per-tier

| Group | N | Recall@5 | Recall@10 | MRR | Correct-heading |
|---|---|---|---|---|---|
| hard | 85 | 0.941 | 0.941 | 0.795 | 0.947 |
| realistic | 33 | 1.000 | 1.000 | 0.977 | 0.400 |
| diagnostic | 7 | 0.571 | 1.000 | 0.568 | n/a |

### Per-query breakdown

| ID | Query | Rank of first expected | Anti in top-5? |
|---|---|---|---|
| C1 | which machine handles streaming my films and shows to the telly | 1 | no |
| C2 | which box handles name resolution at my parents' place | — | no |
| C3 | where does the long-running autonomous agent live | 2 | no |
| C4 | which host mints the short-lived credentials my agents use to SSH around | 4 | no |
| C5 | where are the offsite copies of my git repositories kept | 5 | no |
| C6 | what runs the retro gaming console plugged into the TV | 1 | no |
| C7 | which server scans and files my paperwork | 1 | no |
| C8 | what do requests from outside hit first before reaching my services | — | no |
| C9 | which coffee did Nadine actually enjoy | 3 | no |
| C10 | the beans with the citrus and floral notes | 2 | yes |
| C11 | who should make decisions on my behalf if I am incapacitated | 1 | yes |
| C12 | which of my data would actually hurt to lose | 1 | no |
| C13 | what should be switched off rather than kept running after I am gone | 1 | no |
| C14 | something to get my dad | 1 | yes |
| C15 | first knife for a kid | 1 | no |
| H1 | how does the annual optical disc snapshot fit into the photo backup plan | 1 | — |
| H2 | what cloud service holds the second copy of our photos | 1 | — |
| H3 | why did we end up without one central JPEG library | 1 | — |
| H4 | what IP addresses are assigned across the homelab | 1 | — |
| H5 | how are the media drives laid out | 1 | — |
| H6 | authoritative list of VMs and containers on the main Proxmox node | 1 | — |
| H7 | which clients are currently connected to the VPN | 1 | — |
| H8 | what to check first when name resolution breaks | 1 | — |
| H9 | how does the docker host authenticate to pull the deployment repo | 1 | — |
| H10 | what were the main design decisions behind the deployment pipeline | 1 | — |
| F1 | zfs-load-key-cryptpool.service failed at boot | 1 | no |
| F2 | net.core.default_qdisc fq | 1 | no |
| F3 | what is the iscsiadm sendtargets command against the portal on 3260 | 1 | no |
| F4 | 127.0.0.1:8847 healthz connection refused | — | — |
| F5 | ssh -N -L 1455:localhost:1455 | 1 | no |
| F6 | why does container uid 65532 end up as 165531 on the host | 2 | — |
| F7 | 21116 udp forward | 1 | no |
| F8 | 192.168.31.230 | — | yes |
| F9 | which sshd_config.d drop-in sets TrustedUserCAKeys | 1 | no |
| F10 | vm.swappiness=10 | 1 | — |
| F11 | telegram-notify@ template unit OnFailure | 1 | no |
| F12 | mnt-tmvol.mount | 1 | no |
| F13 | trivy-fleet-audit.timer | 1 | yes |
| F14 | when does snapraid-scrub.timer actually fire | 1 | yes |
| M1 | how do I build and publish a container image so my own hosts can pull it, arm… | 1 | no |
| M2 | everything that inserts firewall rules ahead of Docker on the edge box | 1 | no |
| M3 | why did resolution keep breaking and what did I change to make it stick | 2 | yes |
| M4 | how dependency bumps get proposed, reviewed, and deliberately skipped | 1 | no |
| M5 | handing the onboard graphics chip to a guest | 1 | no |
| M6 | what should I use to build an interface that runs in the terminal | 1 | no |
| M7 | getting the car's charge level onto my dashboards | 1 | yes |
| M8 | the wall shades we settled on and their fan-deck codes | 1 | no |
| M9 | where are the scan images from the pregnancy | 1 | no |
| M10 | what were we told to buy before the baby arrives | 1 | no |
| M11 | preparing for the maternity nurse visits | 4 | no |
| M12 | recipe books to give her | 1 | no |
| M13 | the modular storage bin designs I bookmarked to print | 1 | no |
| M14 | where do I keep track of which bags I have already bought | 3 | no |
| X1 | what should we do this weekend | 1 | yes |
| X2 | something to put on tonight | 1 | yes |
| X3 | I want to buy something nice for the flat | 2 | yes |
| X4 | what am I meant to be reading | 1 | no |
| X5 | an idea I could actually sit down and build | 4 | no |
| X6 | keeping the machines patched and scanned for holes | 1 | yes |
| X7 | how do I handle people I find hard to deal with | 1 | yes |
| X8 | the general routine for looking after the indoor plants | 5 | yes |
| X9 | how am I going about picking up the language | 1 | no |
| X10 | what should I print next | 1 | no |
| X11 | ways to give an assistant a memory that persists | 2 | no |
| X12 | how would I find out a disk is dying before it takes something with it | 4 | no |
| X13 | the rules for keeping these notes tidy | 1 | yes |
| X14 | the emotional side of becoming a father | 1 | yes |
| X15 | something about accepting what you cannot change | 1 | no |
| X16 | how are we going to handle solids when the time comes | 1 | no |
| X17 | cheaper ways to rent compute | 1 | no |
| H11 | does the mirror box in France tunnel through its host or dial out on its own | 1 | yes |
| H12 | what runs overnight, hour by hour | 1 | yes |
| H13 | what happens if the key server is unreachable when a machine reboots | 3 | yes |
| H14 | which of the two feeds should I actually point the indexer at | 1 | no |
| H15 | how do I choose an ID when I create a new guest | 1 | no |
| H16 | which ports are genuinely reachable from outside rather than just configured | 1 | no |
| H17 | how should the assistant avoid burning tokens when it reads my notes | 1 | no |
| H18 | which guest was actually filling up the backup store | 3 | yes |
| H19 | how do I get an off-site agent talking again after its key drifts | 1 | no |
| H20 | what has changed on the little N100 machine lately | 4 | no |
| H21 | which packages did I deliberately tell the bot to leave alone | 2 | no |
| N1 | Kokuyo Campus notebook buying notes | 1 | yes |
| C16 | which of the plants gets watered on a fixed weekly schedule | 1 | yes |
| C17 | the one I should test with a finger instead of watering to a routine | — | no |
| C18 | where do we get Indonesian takeaway | 1 | yes |
| C19 | the oil we still want to try rather than the one already in the cupboard | 1 | yes |
| C20 | which box holds the break-glass copy of my repos that is still in the flat | 3 | no |
| C21 | how many kicks should I feel in two hours before ringing someone | 2 | no |
| C22 | what to do with the quarterly state payment for the kid instead of spending it | 1 | no |
| C23 | that sweet with the crunchy shell and the jelly middle | 1 | yes |
| C24 | who do I call when a pipe leaks | 1 | yes |
| C25 | should she cut foods out of her diet if he is crying a lot | 1 | yes |
| S1 | how are the DAS shares mounted for the media stack | 6 | yes |
| S2 | what does network-wide DNS filtering run on | 6 | no |
| S3 | how do I keep homelab secrets encrypted at rest | 2 | yes |
| S4 | how are container image updates automated | 7 | yes |
| S5 | how is the browser terminal exposed | 1 | no |
| S6 | the script that sets up my shell on a freshly built machine | 1 | no |
| S7 | letting the parity disk spin down when nothing is using it | 1 | yes |
| D1 | How do I restore a Proxmox backup if BatterNAS is dead? | 1 | — |
| D2 | What's the MergerFS pool layout on BatterProx? | 1 | — |
| D3 | How can I setup DNS + SSL wired up across the homelab? | 1 | — |
| D4 | How do I add a new client to Authelia? | 1 | — |
| D5 | Fix for Cloudflare 522 when NPM isn't forwarding | 1 | — |
| D6 | How to set up Atuin shell history on a new machine | 1 | — |
| D7 | How do I get Telegram alerts when Kopia backup fails? | 1 | — |
| D8 | Which coffees did I rate as rebuy-worthy? | 1 | — |
| D9 | Tasting notes for the Pergamino Alto de Letras | 1 | — |
| D10 | What's our plan for flying with the baby? | 1 | — |
| D11 | Open research questions I still need to resolve about parenting | 1 | — |
| D12 | Babymoov Nutribaby — what did I note about it? | 1 | — |
| D13 | What's my handover model if I can't manage things anymore? | 1 | — |
| D14 | Where is the inventory of all my assets and systems? | 1 | — |
| D15 | Best Anki decks for Spanish vocabulary | 1 | — |
| D16 | How does Hatchdoor generate page URLs? | 1 | — |
| D17 | Markdown features Hatchdoor supports | 1 | — |
| D18 | Tenant support resources in Amsterdam | 1 | — |
| D19 | Geneva airport free WiFi code | 1 | — |
| D20 | Aurélien's political views — quick reference | 1 | — |
| U1 | Where does my Plex media live? | 1 | — |
| U2 | I'm looking for a new smell for the house | 2 | no |
| U3 | How often should I feed my Calathea? | 1 | — |
| U4 | How do I want to update my backup strategy? | 1 | yes |
| U5 | I am travelling by plane with the baby | 1 | yes |
| U6 | How can I reflect on things with my family? | 1 | yes |
