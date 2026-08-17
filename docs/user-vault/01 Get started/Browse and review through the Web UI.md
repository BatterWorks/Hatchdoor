---
tags: [type/tutorial, topic/web-ui]
---

# Browse and review through the Web UI

The Web UI and your agent work on the same Vault. Use the browser to inspect
the agent's change and to navigate notes visually.

1. Open `http://localhost:42824` and enter the web token if prompted.
2. Use the sidebar explorer to open the note the agent changed.
3. Find the added bullet and confirm it belongs under the intended heading.
4. Use the Vault selector to narrow to one Vault, or choose **All Vaults** to
   browse across enabled Vaults.

To find notes yourself, select **Search** in the top bar (or press `/` outside
a text field). Semantic search is the default: use it for ideas and meaning.
Turn on **Keyword mode** when exact wording matters, such as a hostname, tag,
filename, command, or ID.

> [!note]
> Browser search and agent search complement each other. Search in the Web UI when you want to scan results; ask the agent when you want a controlled research or editing workflow.

The UI can also edit notes when the Vault is writable. **New note** creates a
Markdown file; **Edit** and **Save** change one. If those controls are absent,
the Vault is read-only or the deployment is in demo mode.

Hatchdoor understands wikilinks and refreshes its index when Markdown or
attachments change. Keep the Markdown files portable: you can still open them
in another Markdown app at any time.

Finish with [[Understand where your data lives]].



---

Previous: [[Search and change notes with your agent]]
Next: [[Understand where your data lives]]
