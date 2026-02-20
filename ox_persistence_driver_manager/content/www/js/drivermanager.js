
async function fetchDrivers() {
    try {
        // Ensure we request JSON
        const response = await fetch('/drivers/', {
            headers: { 'Accept': 'application/json' }
        });

        if (!response.ok) {
            throw new Error(`Failed to fetch drivers: ${response.status} ${response.statusText}`);
        }

        const data = await response.json();

        // Handle structure: { drivers: [...] }
        const list = data.drivers || [];
        renderDrivers(list);
    } catch (e) {
        console.error(e);
        showError(e.message);
    }
}

function showError(msg) {
    const el = document.getElementById('error-banner');
    el.style.display = 'block';
    el.innerText = msg;
}

function renderDrivers(drivers) {
    const list = document.getElementById('driver-list');
    list.innerHTML = '';

    if (drivers.length === 0) {
        list.innerHTML = '<div style="padding: 1rem;">No drivers found.</div>';
        return;
    }

    drivers.forEach(driver => {
        const isEnabled = driver.state === 'enabled';
        const statusClass = isEnabled ? 'enabled' : 'disabled';
        const btnLabel = isEnabled ? 'Disable' : 'Enable';
        const btnClass = isEnabled ? 'disable' : 'enable';

        // Use display_name if available, fallback to name (friendly name ideally), otherwise ID
        // Currently 'name' returns package name like 'ox_persistence_driver_db_mysql'
        // 'display_name' returns e.g. 'MySQL' if backend works
        const displayName = driver.display_name || driver.name;

        const row = document.createElement('div');
        row.className = 'driver-row';

        let infoButton = '';
        if (driver.metadata) {
            infoButton = `
                <button class="driver-icon-btn btn-info" data-metadata='${driver.metadata.replace(/'/g, "&apos;")}' title="Info">
                    <svg viewBox="0 0 24 24" width="18" height="18" fill="currentColor">
                        <path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm1 15h-2v-6h2v6zm0-8h-2V7h2v2z"/>
                    </svg>
                </button>
            `;
        }

        row.innerHTML = `
            <div class="driver-status">
                <div class="status-dot ${statusClass}" title="${driver.state}"></div>
            </div>
            <div class="driver-info">
                <div class="driver-name">
                    ${displayName}
                    ${infoButton}
                </div>
                <div class="driver-id">${driver.id}</div>
            </div>
            <div class="driver-actions">
                <button class="btn-toggle ${btnClass}" data-id="${driver.id}">
                    ${btnLabel}
                </button>
            </div>
        `;
        list.appendChild(row);
    });

    // Attach event listeners for buttons
    document.querySelectorAll('.btn-toggle').forEach(btn => {
        btn.addEventListener('click', async (e) => {
            e.preventDefault();
            // Disable button to prevent double-click
            e.target.disabled = true;
            e.target.innerText = '...';

            const id = e.target.getAttribute('data-id');
            await toggleDriver(id);
        });
    });

    // Attach event listeners for info buttons
    document.querySelectorAll('.btn-info').forEach(btn => {
        btn.addEventListener('click', (e) => {
            e.preventDefault();
            const meta = e.currentTarget.getAttribute('data-metadata');
            showMetadata(meta);
        });
    });
}

function showMetadata(metaStr) {
    let content = metaStr;
    try {
        // Try parsing as JSON for prettier display
        const json = JSON.parse(metaStr);
        content = '<table class="meta-table">';
        for (const [key, value] of Object.entries(json)) {
            content += `<tr><th>${key}</th><td>${value}</td></tr>`;
        }
        content += '</table>';
    } catch (e) {
        // Not JSON, just show as text
        content = `<pre>${metaStr}</pre>`;
    }

    const modal = document.createElement('div');
    modal.className = 'modal-overlay';
    modal.innerHTML = `
        <div class="modal">
            <div class="modal-header">
                <h3>Driver Information</h3>
                <button class="btn-close">&times;</button>
            </div>
            <div class="modal-content">
                ${content}
            </div>
        </div>
    `;

    document.body.appendChild(modal);

    const close = () => document.body.removeChild(modal);
    modal.querySelector('.btn-close').addEventListener('click', close);
    modal.addEventListener('click', (e) => {
        if (e.target === modal) close();
    });
}


async function toggleDriver(id) {
    try {
        const response = await fetch(`/drivers/${id}`, {
            method: 'POST',
            headers: { 'Accept': 'application/json' }
        });
        if (!response.ok) {
            const text = await response.text();
            throw new Error(`Failed to toggle: ${text || response.statusText}`);
        }
        // Refresh list on success
        await fetchDrivers();
    } catch (e) {
        console.error(e);
        showError(e.message);
    }
}

// Initialize
fetchDrivers();
