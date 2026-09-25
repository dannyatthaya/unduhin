// Which `webRequest` resource types the capture listeners observe.
//
// A listener on `<all_urls>` with no type filter runs for every request in
// every tab — each image, script, stylesheet, font and beacon — and
// `extraHeaders` makes each of those calls more expensive for Chrome. None
// of those can be a download or a stream manifest, which only ever arrive
// as a navigation (`main_frame` / `sub_frame`), a plugin load (`object`),
// a fetch / XHR (`xmlhttprequest`), a media element load (`media`), or
// Chrome's catch-all `other` (which includes download requests).

export const CAPTURE_RESOURCE_TYPES: chrome.webRequest.ResourceType[] = [
  "main_frame",
  "sub_frame",
  "object",
  "xmlhttprequest",
  "media",
  "other",
];
