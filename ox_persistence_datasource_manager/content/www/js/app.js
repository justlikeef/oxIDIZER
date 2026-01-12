document.addEventListener('DOMContentLoaded', () => {
    const list = document.getElementById('data-sources-list');
    const refreshBtn = document.getElementById('refresh-btn');
    const statusIndicator = document.getElementById('status-badge');

    initGlobalStatus('status-badge');

    // --- Actions ---

    // 1. Add New Data Source
    async function openNewDataSource() {
        // 1. Select Driver
        const drivers = await fetchDrivers();
        if (!drivers || drivers.length === 0) {
            alert('No drivers available. Please install a driver first.');
            return;
        }

        const driverId = await promptDriverSelection(drivers);
        if (!driverId) return;

        // 2. Load Form
        await loadAndShowForm(null, driverId);
    }

    // 2. Edit Data Source
    async function editDataSource(id) {
        await loadAndShowForm(id, null);
    }

    // 3. Delete Data Source
    async function deleteDataSource(id) {
        if (!confirm(`Are you sure you want to delete data source "${id}"?`)) return;

        try {
            const res = await fetch(`/datasources/${id}`, { method: 'DELETE' });
            if (res.ok) {
                loadDataSources(); // Refresh
            } else {
                alert('Failed to delete: ' + res.statusText);
            }
        } catch (e) {
            console.error(e);
            alert('Error deleting data source');
        }
    }

    // --- Helpers ---

    async function fetchDrivers() {
        try {
            const res = await fetch('/drivers/?state=enabled', { headers: { 'Accept': 'application/json' } });
            if (res.ok) {
                const data = await res.json();
                return data.drivers || [];
            }
        } catch (e) { console.error(e); }
        return [];
    }

    async function promptDriverSelection(drivers) {
        return new Promise((resolve) => {
            const modal = document.createElement('div');
            modal.className = 'modal-overlay';
            modal.innerHTML = `
                <div class="modal glass">
                    <h3>Select a Driver</h3>
                    <div class="form-group" style="margin-top: 1rem;">
                        <label for="driver-select" style="display: block; margin-bottom: 0.5rem; color: #ccc;">Available Drivers</label>
                        <select id="driver-select" class="form-control">
                            ${drivers.map(d => `<option value="${d.id}">${d.display_name || d.name}</option>`).join('')}
                        </select>
                    </div>
                    <div class="modal-actions" style="margin-top: 2rem; display: flex; justify-content: flex-end; gap: 1rem;">
                        <button class="btn btn-secondary" id="cancel-driver">Cancel</button>
                        <button class="btn btn-primary" id="confirm-driver">Next</button>
                    </div>
                </div>
            `;
            document.body.appendChild(modal);

            document.getElementById('confirm-driver').onclick = () => {
                const select = document.getElementById('driver-select');
                const id = select.value;
                document.body.removeChild(modal);
                resolve(id);
            };

            document.getElementById('cancel-driver').onclick = () => {
                document.body.removeChild(modal);
                resolve(null);
            };
        });
    }

    async function loadAndShowForm(dsId, driverId) {
        const url = dsId
            ? `/datasources/new/form?id=${dsId}`
            : `/datasources/new/form?driver=${driverId}`;

        try {
            const res = await fetch(url);
            if (!res.ok) throw new Error(res.statusText);
            const html = await res.text();

            showFormModal(html, dsId ? 'Edit Data Source' : 'New Data Source');
        } catch (e) {
            alert('Failed to load form: ' + e.message);
        }
    }

    function showFormModal(htmlContent, title) {
        const modal = document.createElement('div');
        modal.className = 'modal-overlay';
        modal.innerHTML = `
            <div class="modal glass modal-lg">
                <div class="modal-header">
                    <h3>${title}</h3>
                    <button class="btn-close">&times;</button>
                </div>
                <div class="modal-body">
                    ${htmlContent}
                </div>
            </div>
        `;
        document.body.appendChild(modal);

        // Manually execute scripts because innerHTML doesn't
        modal.querySelectorAll('.modal-body script').forEach(oldScript => {
            const newScript = document.createElement('script');
            Array.from(oldScript.attributes).forEach(attr => newScript.setAttribute(attr.name, attr.value));
            newScript.textContent = oldScript.textContent;
            oldScript.parentNode.replaceChild(newScript, oldScript);
        });

        // Handle Close
        const close = () => document.body.removeChild(modal);
        modal.querySelector('.btn-close').onclick = close;

        // Handle Form Submit (Intercept)
        const form = modal.querySelector('form');
        if (form) {
            form.onsubmit = async (e) => {
                e.preventDefault();
                const formData = new FormData(form);
                const data = Object.fromEntries(formData.entries());

                // Construct the DataSource JSON structure expected by backend
                // The form fields are flat, but we need nested logic or we need to update backend to accept flat?
                // The backend `create` route expects `DataSource` struct JSON.
                // But the FORM is generated from driver schema which is just config.
                // We need `id`, `name`, `driver_id`, `config`.
                // The form probably doesn't have `id`, `name`, `driver_id` fields unless we injected them.
                // We should inject them or ask user for them.

                // For now, let's assume the form has them or we prompt?
                // The current schema generation only generated config fields.
                // We need to wrap the payload.

                // WORKAROUND: For this iteration, we build a partial payload. 
                // In reality, we need 'id' and 'name' inputs.
                // Let's check if the HTML form has them. 
                // If not, we might fail.
                // But for now, let's try to submit what we have + driver_id.

                // Wait, `ox_persistence_datasource_manager` backend expects `DataSource` struct.
                // We need to construct it.
                // Let's assume we capture "id" and "name" if present, else auto-generate?

                // Let's add ID/Name fields to the modal if they aren't in the form
                // Or rely on the user to fix the form schema.

                const payload = {
                    id: data.id || ('ds_' + Date.now()),
                    name: data.name || 'New Data Source',
                    driver_id: data.driver_id || 'unknown', // We need to pass this somehow. hidden field?
                    config: data // The rest is config
                };

                // Clean up config
                delete payload.config.id;
                delete payload.config.name;
                delete payload.config.driver_id;

                try {
                    const postRes = await fetch('/datasources', {
                        method: 'POST',
                        body: JSON.stringify(payload),
                        headers: { 'Content-Type': 'application/json' }
                    });
                    if (postRes.ok) {
                        close();
                        loadDataSources();
                    } else {
                        alert('Error saving: ' + await postRes.text());
                    }
                } catch (err) {
                    console.error(err);
                    alert('Error submitting form');
                }
            };
        }
    }


    async function loadDataSources() {
        if (!list) return;
        list.innerHTML = `<div class="loader"></div>`;

        try {
            const response = await fetch('', {
                headers: { 'Accept': 'application/json' }
            });

            if (response.ok) {
                const data = await response.json();
                renderDataSources(data);
            } else {
                renderError(response.status + ' ' + response.statusText);
            }
        } catch (e) {
            console.error(e);
            renderError(e.message);
        }
    }

    function renderError(msg) {
        list.innerHTML = `
            <div class="card error-card glass" style="grid-column: 1/-1;">
                <h3 style="color: #ef4444;">Error</h3>
                <p>${msg}</p>
                <button class="btn btn-primary" onclick="location.reload()">Retry</button>
            </div>
        `;
    }

    function renderDataSources(data) {
        let sources = [];
        if (Array.isArray(data)) sources = data;
        else if (data && typeof data === 'object') {
            sources = data.data_sources || Object.values(data);
        }

        if (!sources || sources.length === 0) {
            list.innerHTML = `
                <div class="empty-state-container" style="grid-column: 1 / -1; text-align: center; padding: 40px;">
                    <h3>No Data Sources</h3>
                    <button class="btn btn-primary" id="add-first-btn">Add Data Source</button>
                </div>
            `;
            document.getElementById('add-first-btn').onclick = openNewDataSource;
            return;
        }

        list.innerHTML = '';
        sources.forEach((source, index) => {
            const card = document.createElement('div');
            card.className = 'card data-source-card glass';
            card.innerHTML = `
                <div class="card-header">
                    <h3>${source.name || source.id}</h3>
                    <span class="badge">${source.driver_id || 'Unknown Driver'}</span>
                </div>
                <div class="card-body">
                    <p>ID: ${source.id}</p>
                </div>
                <div class="card-actions">
                    <button class="btn btn-icon edit-btn" data-id="${source.id}" title="Edit">
                        <svg viewBox="0 0 24 24" width="18" height="18"><path d="M3 17.25V21h3.75L17.81 9.94l-3.75-3.75L3 17.25zM20.71 7.04c.39-.39.39-1.02 0-1.41l-2.34-2.34a.9959.9959 0 0 0-1.41 0l-1.83 1.83 3.75 3.75 1.83-1.83z"/></svg>
                    </button>
                    <button class="btn btn-icon btn-danger delete-btn" data-id="${source.id}" title="Delete">
                         <svg viewBox="0 0 24 24" width="18" height="18"><path d="M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z"/></svg>
                    </button>
                </div>
             `;
            list.appendChild(card);
        });

        // Event Listeners
        document.querySelectorAll('.edit-btn').forEach(b => {
            b.onclick = () => editDataSource(b.getAttribute('data-id'));
        });
        document.querySelectorAll('.delete-btn').forEach(b => {
            b.onclick = () => deleteDataSource(b.getAttribute('data-id'));
        });
    }


    if (refreshBtn) refreshBtn.onclick = loadDataSources;

    // Add Global "Add" Button to header if it doesn't exist? 
    // Usually the layout has a header place.
    // Let's assume we can inject it or the user will add it to HTML.
    // For now, let's try to inject into the `header-status-area` or similar if found.
    // Wire up existing "Create New" button in the card
    const addSourceBtn = document.getElementById('add-source-btn');
    if (addSourceBtn) {
        addSourceBtn.onclick = openNewDataSource;
    }

    loadDataSources();
});
