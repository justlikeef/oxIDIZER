/**
 * common.js - Shared logic for oxIDIZER modules
 */

let failedPings = 0;

/**
 * Initialize global server status ping poller
 * @param {string} badgeId - ID of the status badge element
 */
function initGlobalStatus(badgeId = 'status-badge') {
    const badge = document.getElementById(badgeId);
    if (!badge) return;

    let ws = null;
    let pollInterval = null;
    let wsRetryCount = 0;
    const maxWsRetries = 3;
    let heartbeatInterval = null;

    const updateBadge = (status) => {
        badge.className = 'status-badge';
        if (status === 'online') {
            badge.textContent = 'ONLINE';
            badge.classList.add('status-ok');
        } else if (status === 'alert') {
            badge.textContent = 'ALERT';
            badge.classList.add('status-warn');
        } else {
            badge.textContent = 'OFFLINE';
            badge.classList.add('status-error');
        }
    };

    const startHttpPolling = () => {
        if (pollInterval) return;
        console.log('Falling back to HTTP polling for status');

        const checkPing = async () => {
            try {
                const controller = new AbortController();
                const timeoutId = setTimeout(() => controller.abort(), 1000);

                const response = await fetch('/ping/', {
                    headers: { 'Accept': 'application/json' },
                    signal: controller.signal
                });
                clearTimeout(timeoutId);

                if (response.ok) {
                    const data = await response.json();
                    if (data.response === 'pong') {
                        failedPings = 0;
                        updateBadge('online');
                    } else {
                        throw new Error('Invalid pong');
                    }
                } else {
                    throw new Error('Ping failed');
                }
            } catch (e) {
                failedPings++;
                updateBadge(failedPings < 3 ? 'alert' : 'offline');
            }
        };

        checkPing();
        pollInterval = setInterval(checkPing, 3000);
    };

    const connectWS = () => {
        const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
        const wsUrl = `${protocol}//${window.location.host}/ws/ping/`;

        try {
            ws = new WebSocket(wsUrl);

            ws.onopen = () => {
                console.log('WebSocket connected to /ws/ping/');
                failedPings = 0;
                wsRetryCount = 0;
                updateBadge('online');

                if (pollInterval) {
                    clearInterval(pollInterval);
                    pollInterval = null;
                }

                if (heartbeatInterval) clearInterval(heartbeatInterval);
                heartbeatInterval = setInterval(() => {
                    if (ws.readyState === WebSocket.OPEN) {
                        ws.send('ping');
                    }
                }, 5000);
            };

            ws.onmessage = (event) => {
                // Heartbeat response or status updates could be handled here
            };

            ws.onclose = () => {
                if (heartbeatInterval) {
                    clearInterval(heartbeatInterval);
                    heartbeatInterval = null;
                }

                if (wsRetryCount < maxWsRetries) {
                    wsRetryCount++;
                    setTimeout(connectWS, 2000);
                } else {
                    startHttpPolling();
                }
            };

            ws.onerror = () => {
                ws.close();
            };
        } catch (e) {
            startHttpPolling();
        }
    };

    connectWS();
}
