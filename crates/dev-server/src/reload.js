// Dev only (scripts/dev.sh adds it to the page; production never serves
// it). Reloads the page after a rebuild of the client, or after the dev
// server restarts. A file, not an inline script: the page's CSP allows
// `script-src 'self'` only, and `connect-src 'self'` covers this stream.
// Each event's data is the generation of the bundle being served.
let seen = null;
function check(event) {
    if (seen !== null && seen !== event.data) {
        location.reload();
    }
    seen = event.data;
}
const events = new EventSource("/__agora-web-dev/events");
events.addEventListener("built", check);
events.addEventListener("failed", (event) => {
    check(event);
    console.error("agora-web dev: the rebuild failed; see the dev.sh terminal. The page still runs the last good build.");
});
