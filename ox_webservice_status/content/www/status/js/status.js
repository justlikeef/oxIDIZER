document.addEventListener('DOMContentLoaded', () => {
    fetchStatus();
    startPingPoller();
    initAutoRefresh();
});

let failedPings = 0;
let refreshTimer = null;

function initAutoRefresh() {
    const toggle = document.getElementById('auto-refresh-toggle');
    const intervalInput = document.getElementById('refresh-interval');

    if (!toggle || !intervalInput) return;

    const update = () => {
        if (refreshTimer) clearInterval(refreshTimer);
        refreshTimer = null;

        if (toggle.checked) {
            let delay = parseInt(intervalInput.value, 10);
            if (isNaN(delay)) delay = 30;
            if (delay < 5) { delay = 5; intervalInput.value = 5; }
            if (delay > 300) { delay = 300; intervalInput.value = 300; }

            refreshTimer = setInterval(fetchStatus, delay * 1000);
        }
    };

    toggle.addEventListener('change', update);
    intervalInput.addEventListener('change', update);

    // Initialize state
    update();
}

async function startPingPoller() {
    const badge = document.getElementById('status-badge');

    // Initial check
    await checkPing(badge);

    // Poll every 3 seconds
    setInterval(() => checkPing(badge), 3000);
}

async function checkPing(badge) {
    if (!badge) return;

    try {
        const controller = new AbortController();
        const timeoutId = setTimeout(() => controller.abort(), 1000); // 1s timeout

        const response = await fetch('/ping/', {
            headers: { 'Accept': 'application/json' },
            signal: controller.signal
        });
        clearTimeout(timeoutId);

        if (response.ok) {
            const data = await response.json();
            if (data.response === 'pong') {
                // Success
                failedPings = 0;
            } else {
                throw new Error('Invalid pong response');
            }
        } else {
            throw new Error('Ping failed');
        }
    } catch (e) {
        failedPings++;
    }

    // State transitions
    badge.className = 'status-badge'; // Reset classes
    if (failedPings === 0) {
        badge.textContent = 'ONLINE';
        badge.classList.add('status-ok');
        badge.style.backgroundColor = '';
    } else if (failedPings < 3) {
        badge.textContent = 'ALERT';
        badge.classList.add('status-warn');
        badge.style.backgroundColor = '';
    } else {
        badge.textContent = 'OFFLINE';
        badge.classList.add('status-error');
        badge.style.backgroundColor = '';
    }
}

async function fetchStatus() {
    try {
        const response = await fetch('?format=json', {
            headers: {
                'Accept': 'application/json'
            }
        });

        if (!response.ok) {
            throw new Error(`HTTP error! status: ${response.status}`);
        }

        const data = await response.json();
        render(data);
    } catch (error) {
        console.error('Error fetching status:', error);
        document.body.innerHTML = `<div class="container"><h1>Error fetching status</h1><p>${error.message}</p></div>`;
    }
}

function render(data) {
    // Header
    const sys = data.system || {};
    // renderUptime removed, now handled in metrics


    // System Info
    const sysEl = document.getElementById('system-info');
    sysEl.innerHTML = `
        <div class="stat-row"><span class="label">Hostname</span><span class="value">${escapeHtml(sys.host_name || 'N/A')}</span></div>
        <div class="stat-row"><span class="label">OS Version</span><span class="value">${escapeHtml(sys.os_version || 'N/A')}</span></div>
        <div class="stat-row"><span class="label">Kernel</span><span class="value">${escapeHtml(sys.kernel_version || 'N/A')}</span></div>
    `;

    // Resources
    // Resources
    const resEl = document.getElementById('resources-info');
    const mem = sys.memory || { used: 0, total: 1, swap_used: 0, swap_total: 0 };
    const load = sys.load_average || { one: 0, five: 0, fifteen: 0 };

    const memPct = (mem.used / mem.total) * 100;
    const gb = 1073741824;

    resEl.innerHTML = `
        <div class="stat-row"><span class="label">CPU Cores</span><span class="value">${sys.cpu_count || 0}</span></div>
        <div class="stat-row"><span class="label">Load Avg</span><span class="value">${load.one.toFixed(2)} / ${load.five.toFixed(2)} / ${load.fifteen.toFixed(2)}</span></div>
        
        <div style="margin-top: 1.5rem;">
            <div class="stat-row"><span class="label">Memory</span><span class="value">${(mem.used / gb).toFixed(1)} / ${(mem.total / gb).toFixed(1)} GB</span></div>
            <div class="progress-bar">
                <div class="progress-fill" style="width: ${memPct}%"></div>
            </div>
             <div class="stat-row" style="margin-top:0.5rem;"><span class="label">Swap</span><span class="value">${(mem.swap_used / gb).toFixed(1)} / ${(mem.swap_total / gb).toFixed(1)} GB</span></div>
        </div>
    `;

    // Storage
    const storageEl = document.getElementById('storage-info');
    const disks = sys.disks || [];
    if (disks.length > 0) {
        storageEl.innerHTML = disks.map(d => {
            const pct = d.total_space > 0 ? (100 - (d.available_space / d.total_space * 100)) : 0;
            const totalGb = d.total_space / gb;
            const usedGb = (d.total_space - d.available_space) / gb;
            return `
            <div class="storage-item">
                <div class="storage-header">
                    <div class="storage-info">
                        <strong>${escapeHtml(d.name)}</strong>
                        <span>${escapeHtml(d.mount_point)}</span>
                    </div>
                    <div class="storage-stats">
                        <div class="percent">${pct.toFixed(1)}%</div>
                        <div class="details">${usedGb.toFixed(1)} / ${totalGb.toFixed(1)} GB</div>
                    </div>
                </div>
                <div class="progress-bar">
                    <div class="progress-fill" style="width: ${pct}%"></div>
                </div>
            </div>
            `;
        }).join('');
    } else {
        storageEl.innerHTML = '<p class="label">No storage devices found</p>';
    }

    // Server Metrics
    // Server Metrics
    const metricsEl = document.getElementById('server-metrics');
    let serverMetrics = data.server ? data.server.metrics : null;
    const process = data.server ? data.server.process : null;

    if (!serverMetrics) serverMetrics = {};

    if (process) {
        serverMetrics["Process Uptime"] = formatUptime(process.uptime_seconds || 0);
        serverMetrics["Memory Usage"] = formatBytes(process.memory_bytes || 0);
        serverMetrics["CPU Usage"] = (process.cpu_usage || 0).toFixed(2) + '%';
    } else {
        serverMetrics["Process Uptime"] = "N/A";
    }

    if (Object.keys(serverMetrics).length > 0) {
        metricsEl.innerHTML = renderMetricsObject(serverMetrics, true);
    } else {
        metricsEl.innerHTML = '<div class="card"><p>No metrics available</p></div>';
    }

    // Pipeline Routing (NEW)
    renderPipelineRouting(data.pipeline_routing, data.configurations);

    // Configurations
    const configEl = document.getElementById('configurations');
    if (data.configurations) {
        configEl.innerHTML = renderConfigObject(data.configurations);
    } else {
        configEl.innerHTML = '<p>No configurations available</p>';
    }
}

function formatBytes(bytes) {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
}

function formatUptime(seconds) {
    const y = Math.floor(seconds / 31536000);
    seconds %= 31536000;
    const d = Math.floor(seconds / 86400);
    seconds %= 86400;
    const h = Math.floor(seconds / 3600);
    seconds %= 3600;
    const m = Math.floor(seconds / 60);
    const s = seconds % 60;

    const parts = [];
    if (y > 0) parts.push(`${y}y`);
    if (d > 0) parts.push(`${d}d`);
    if (h > 0) parts.push(`${h}h`);
    if (m > 0) parts.push(`${m}m`);
    parts.push(`${s}s`);

    return parts.join(' ');
}

function renderMetricsObject(obj, isRoot = false) {
    if (!obj || typeof obj !== 'object') return '';

    // If root, we split top-level keys into cards
    if (isRoot) {
        let html = '';
        let generalMetrics = '';

        for (const [key, value] of Object.entries(obj)) {
            if (value && typeof value === 'object' && !Array.isArray(value)) {
                // Complex object -> Card
                html += `
                 <div class="card">
                    <h3>${escapeHtml(key)}</h3>
                    ${renderNestedMetrics(value)}
                 </div>
                 `;
            } else {
                // Primitive or Array -> General Metrics
                generalMetrics += renderMetricRow(key, value);
            }
        }

        if (generalMetrics) {
            html = `
            <div class="card">
                <h3>General</h3>
                ${generalMetrics}
            </div>
            ` + html;
        }
        return html;
    }

    return renderNestedMetrics(obj);
}

function renderNestedMetrics(value) {
    if (!value || typeof value !== 'object') return escapeHtml(String(value));

    // Array
    if (Array.isArray(value)) {
        return value.map((v, i) => `
            <div style="margin-top: 0.5rem;">
                 <h4 style="margin-bottom: 0.25rem;">[${i}]</h4>
                 <div class="nested-metrics">
                    ${renderNestedMetrics(v)}
                 </div>
            </div>
         `).join('');
    }

    // Object
    let html = '';
    for (const [k, v] of Object.entries(value)) {
        if (v && typeof v === 'object') {
            html += `
             <details style="margin-top: 0.5rem; margin-bottom: 0.5rem;">
                <summary style="cursor: pointer; font-weight: bold;">${escapeHtml(k)}</summary>
                <div class="nested-metrics" style="padding-left: 1rem; margin-top: 0.5rem;">
                    ${renderNestedMetrics(v)}
                </div>
             </details>
             `;
        } else {
            html += renderMetricRow(k, v);
        }
    }
    return html;
}

function renderMetricRow(key, value) {
    return `<div class="stat-row"><span class="label">${escapeHtml(key)}</span><span class="value">${escapeHtml(String(value))}</span></div>`;
}

function renderConfigObject(obj) {
    if (!obj) return '';

    let items = [];
    if (Array.isArray(obj)) {
        items = obj;
    } else if (typeof obj === 'object') {
        // Fallback for key-value map
        items = Object.entries(obj).map(([k, v]) => ({ name: k, config: v }));
    } else {
        return '';
    }

    // Filter
    items = items.filter(item => {
        const name = item.name || item.id || '';
        return !name.endsWith('_Router') && name !== 'ox_pipeline_router';
    });

    // Sort by name
    items.sort((a, b) => {
        const na = a.name || a.id || '';
        const nb = b.name || b.id || '';
        return na.localeCompare(nb);
    });

    let html = '';
    for (const item of items) {
        const name = item.name || item.id || 'Unknown Module';
        const configData = item.config || item.params || item; // Fallback to whole item if no config/params

        html += `
        <details class="card" style="margin-bottom: 1rem;">
            <summary style="cursor: pointer; font-weight: bold; color: var(--accent); list-style: none;">${escapeHtml(name)}</summary>
            <div style="margin-top: 1rem;">
                ${renderNestedMetrics(configData)}
            </div>
        </details>
        `;
    }

    if (html === '') return '<p>No other module configurations.</p>';

    return html;
}

function renderPipelineRouting(routing, configurations) {
    // ... container creation code ...
    // Note: I need to preserve the container check code which is before this. 
    // I will replace the function body from start to the sorting logic.

    let container = document.getElementById('pipeline-routing-section');
    if (!container) {
        const configEl = document.getElementById('configurations');
        const headings = document.getElementsByTagName('h2');
        let configH2 = null;
        for (let h of headings) {
            if (h.textContent === 'Configurations') configH2 = h;
        }

        container = document.createElement('div');
        container.id = 'pipeline-routing-section';

        const header = document.createElement('h2');
        header.textContent = 'Pipeline Routing';

        if (configH2) {
            configH2.parentNode.insertBefore(header, configH2);
            configH2.parentNode.insertBefore(container, configH2);
        } else {
            document.querySelector('.container').appendChild(header);
            document.querySelector('.container').appendChild(container);
        }
    }

    if (!routing) {
        container.innerHTML = '<p>No routing info available</p>';
        return;
    }

    let html = '<div class="card" style="overflow-x: auto;"><table class="pipeline-table">';
    html += '<thead><tr class="pipeline-header">';
    html += '<th>Phase</th>';
    html += '<th>Router</th>';
    html += '<th>Routes</th>';
    html += '</tr></thead><tbody>';

    // Sort phases based on execution order from config
    let phaseOrder = [];
    if (configurations && configurations.ox_webservice && configurations.ox_webservice.pipeline && configurations.ox_webservice.pipeline.phases) {
        // phases is an array of objects: [{PhaseName: RouterName}, ...]
        phaseOrder = configurations.ox_webservice.pipeline.phases.map(p => Object.keys(p)[0]);
    } else {
        // Fallback: No known order, use alphabetical
        phaseOrder = [];
    }

    const keys = Object.keys(routing).sort((a, b) => {
        if (phaseOrder.length > 0) {
            let indexA = phaseOrder.indexOf(a);
            let indexB = phaseOrder.indexOf(b);

            // If both found, sort by index
            if (indexA !== -1 && indexB !== -1) {
                return indexA - indexB;
            }
            // If one found, it comes first
            if (indexA !== -1) return -1;
            if (indexB !== -1) return 1;
        }
        // If neither found (or no order defined), do not sort (keep original order)
        return 0;
    });

    for (const phase of keys) {
        const router = routing[phase];
        let routerName = 'Unknown';
        let routeCount = 0;
        let details = '';

        if (typeof router === 'string') {
            routerName = router;
            details = '<span class="error-text">Error</span>';
        } else if (typeof router === 'object') {
            const rInstance = router.router_instance || 'Unknown';
            routerName = escapeHtml(rInstance);

            // Embed config
            if (router.config) {
                let routeInfo = '';
                if (router.config.routes && Array.isArray(router.config.routes)) {
                    routeInfo = ` (${router.config.routes.length} routes)`;
                } else {
                    routeInfo = ' (0 routes)';
                }

                details = `
                 <details>
                    <summary class="router-config-summary">
                        Show Config ${routeInfo}
                    </summary>
                    <div class="router-config-container">
                        ${renderNestedMetrics(router.config)}
                        ${(!router.config || Object.keys(router.config).length === 0) ? '<div class="router-config-empty">Empty Configuration Object</div>' : ''}
                    </div>
                 </details>
                 `;
            } else {
                details = '<small>No Config</small>';
            }
        }

        // Row 1: Basic Info
        html += `<tr class="pipeline-phase-row">`;
        html += `<td><strong>${escapeHtml(phase)}</strong></td>`;
        html += `<td>${routerName}</td>`;
        html += `<td></td>`;
        html += `</tr>`;

        // Row 2: Config (Full Width)
        html += `<tr class="pipeline-config-row">`;
        html += `<td colspan="3" class="pipeline-config-cell">`;
        html += details;
        html += `</td></tr>`;
    }
    html += '</tbody></table></div>';

    container.innerHTML = html;
}

function escapeHtml(unsafe) {
    if (unsafe === undefined || unsafe === null) return '';
    return String(unsafe)
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
        .replace(/"/g, "&quot;")
        .replace(/'/g, "&#039;");
}
