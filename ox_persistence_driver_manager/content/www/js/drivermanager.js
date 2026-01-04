
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
    const grid = document.getElementById('driver-grid');
    grid.innerHTML = '';

    if (drivers.length === 0) {
        grid.innerHTML = '<div style="padding: 1rem;">No drivers found.</div>';
        return;
    }

    drivers.forEach(driver => {
        const statusClass = driver.state === 'enabled' ? 'status-ok' : 'status-error';
        const btnLabel = driver.state === 'enabled' ? 'Disable' : 'Enable';
        const btnClass = driver.state === 'enabled' ? 'btn-disable' : 'btn-enable'; // Optional styling

        const card = document.createElement('div');
        card.className = `card ${statusClass}`;

        // Note: Regular JSON API does not currently return version/description metadata.
        // We render what we have available in the basic struct.

        const libName = `lib${driver.name}.so`;
        const dirPath = driver.library_path ? driver.library_path : 'target/debug';
        const fullPath = `${dirPath}/${libName}`; // Simplified path construction

        card.innerHTML = `
            <div class="card-header">
                <div class="status-badge ${statusClass}">${driver.state.toUpperCase()}</div>
                <div class="card-title">${driver.id}</div> 
            </div>
            <div class="card-content">
                <div class="kv-row">
                    <span class="kv-key">Library</span>
                    <span class="kv-value" title="${fullPath}">${libName}</span> 
                </div>
                <div class="actions" style="margin-top: 1rem; text-align: right;">
                     <button class="btn toggle-btn ${btnClass}" 
                             data-id="${driver.id}">
                        ${btnLabel}
                    </button>
                </div>
            </div>
        `;
        grid.appendChild(card);
    });

    // Attach event listeners for buttons
    document.querySelectorAll('.toggle-btn').forEach(btn => {
        btn.addEventListener('click', async (e) => {
            e.preventDefault();
            // Disable button to prevent double-click
            e.target.disabled = true;
            e.target.innerText = '...';

            const id = e.target.getAttribute('data-id');
            await toggleDriver(id);
        });
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
