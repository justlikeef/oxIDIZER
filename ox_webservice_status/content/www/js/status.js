
document.addEventListener('DOMContentLoaded', () => {
    fetchStatus();
});

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
    renderUptime(data.uptime);

    // System Info
    const sysEl = document.getElementById('system-info');
    sysEl.innerHTML = `
        <div class="stat-row"><span class="label">Hostname</span><span class="value">${escapeHtml(data.host_name || 'N/A')}</span></div>
        <div class="stat-row"><span class="label">System</span><span class="value">${escapeHtml(data.system_name || 'N/A')}</span></div>
        <div class="stat-row"><span class="label">OS Version</span><span class="value">${escapeHtml(data.os_version || 'N/A')}</span></div>
        <div class="stat-row"><span class="label">Kernel</span><span class="value">${escapeHtml(data.kernel_version || 'N/A')}</span></div>
        <div class="stat-row"><span class="label">Config File</span><span class="value">${escapeHtml(data.config_file || 'None')}</span></div>
    `;

    // Resources
    const resEl = document.getElementById('resources-info');
    const memPct = (data.used_memory / data.total_memory) * 100;
    const gb = 1073741824;

    resEl.innerHTML = `
        <div class="stat-row"><span class="label">CPU Cores</span><span class="value">${data.cpu_count}</span></div>
        <div class="stat-row"><span class="label">Load Avg</span><span class="value">${data.load_average.one.toFixed(2)} / ${data.load_average.five.toFixed(2)} / ${data.load_average.fifteen.toFixed(2)}</span></div>
        
        <div style="margin-top: 1.5rem;">
            <div class="stat-row"><span class="label">Memory</span><span class="value">${(data.used_memory / gb).toFixed(1)} / ${(data.total_memory / gb).toFixed(1)} GB</span></div>
            <div class="progress-bar">
                <div class="progress-fill" style="width: ${memPct}%"></div>
            </div>
             <div class="stat-row" style="margin-top:0.5rem;"><span class="label">Swap</span><span class="value">${(data.used_swap / gb).toFixed(1)} / ${(data.total_swap / gb).toFixed(1)} GB</span></div>
        </div>
    `;

    // Storage
    const storageEl = document.getElementById('storage-info');
    if (data.disks && data.disks.length > 0) {
        storageEl.innerHTML = data.disks.map(d => {
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
    const metricsEl = document.getElementById('server-metrics');
    if (data.server_metrics) {
        metricsEl.innerHTML = renderMetricsObject(data.server_metrics, true);
    } else {
        metricsEl.innerHTML = '<div class="card"><p>No metrics available</p></div>';
    }

    // Configurations
    const configEl = document.getElementById('configurations');
    if (data.configurations) {
        configEl.innerHTML = renderConfigObject(data.configurations);
    } else {
        configEl.innerHTML = '<p>No configurations available</p>';
    }
}

function renderUptime(seconds) {
    const el = document.getElementById('uptime-display');
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

    el.textContent = `Uptime: ${parts.join(' ')}`;
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
             <div style="margin-top: 1rem; margin-bottom: 0.5rem;">
                <h4>${escapeHtml(k)}</h4>
                <div class="nested-metrics">
                    ${renderNestedMetrics(v)}
                </div>
             </div>
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
    if (!obj || typeof obj !== 'object') return '';

    let html = '';
    for (const [key, value] of Object.entries(obj)) {
        html += `
        <details class="card" style="margin-bottom: 1rem;">
            <summary style="cursor: pointer; font-weight: bold; color: var(--accent); list-style: none;">${escapeHtml(key)} Configuration</summary>
            <div style="margin-top: 1rem;">
                ${renderNestedMetrics(value)}
            </div>
        </details>
        `;
    }
    return html;
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
