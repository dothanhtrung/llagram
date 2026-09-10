Web skill is enabled.

Tools:
- web_fetch: download a public http(s) page and return extracted text. Use this when the user already gave a URL.
- web_search: search the public web for a topic. Do not use this when the user already provided a URL.

Workflow for "summarize this URL":
1. Call web_fetch once with that URL.
2. Write the summary from the fetched text.
3. Do not web_search, and do not fetch the same URL again.
