document.addEventListener('DOMContentLoaded', () => {
    const list = document.getElementById('data-sources-list');
    const refreshBtn = document.getElementById('refresh-btn');
    const statusIndicator = document.getElementById('status-badge');

    initGlobalStatus('status-badge');

    async function loadDataSources() {
        if (!list) return;

        // Visual loading state
        list.innerHTML = `<div class="loader"></div>`;

        if (statusIndicator) {
            // No longer overriding global status text here
        }

        try {
            const response = await fetch('', {
                headers: {
                    'Accept': 'application/json'
                }
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
            <div class="card error-card glass" style="grid-column: 1/-1; border-color: rgba(239, 68, 68, 0.3);">
                <h3 style="color: #ef4444; margin-bottom: 8px;">Error Loading Data Sources</h3>
                <p style="color: #94a3b8;">${msg}</p>
                <button class="btn btn-primary" style="margin-top: 16px;" onclick="location.reload()">Retry</button>
            </div>
        `;
    }

    function renderDataSources(data) {
        let sources = [];
        // Normalize data
        if (Array.isArray(data)) {
            sources = data;
        } else if (data && typeof data === 'object') {
            sources = data.data_sources || data.datasources || data.items || data.value || [];
            if (sources.length === 0 && Object.keys(data).length > 0 && !data.data_sources && !data.datasources && !data.items && !data.value) {
                sources = Object.values(data);
            }
        }

        if (!sources || sources.length === 0) {
            list.innerHTML = `
                <div class="empty-state-container" style="grid-column: 1 / -1; text-align: center; padding: 40px 0;">
                    <div style="font-size: 3rem; margin-bottom: 20px; opacity: 0.5;">🗄️</div>
                    <h3 style="margin-bottom: 8px;">No Persistence Locations</h3>
                    <p style="color: var(--text-secondary);">Register a new location to begin managing long-term data storage.</p>
                </div>
            `;
            return;
        }

        list.innerHTML = '';

        sources.forEach((source, index) => {
            const name = source.name || source.id || 'Unnamed Source';
            const type = source.type || 'Generic';
            const id = source.id || 'N/A';
            const configPath = source.config_file || source.path || '/etc/ox/sources/...';

            const card = document.createElement('div');
            card.className = 'card data-source-card glass';
            card.style.opacity = '0';
            card.style.transform = 'translateY(20px)';
            card.style.transition = `all 0.5s cubic-bezier(0.16, 1, 0.3, 1) ${index * 0.1}s`;

            card.innerHTML = `
                <div class="card-header">
                    <div style="flex: 1; display: flex; align-items: baseline; gap: 0.75rem; flex-wrap: wrap;">
                        <h3 style="margin: 0;">${name}</h3>
                        <span class="badge" style="background: ${getTypeColor(type)}22; color: ${getTypeColor(type)}; font-size: 0.7rem; padding: 0.15rem 0.4rem; border: 1px solid ${getTypeColor(type)}33;">${type}</span>
                        <p style="width: 100%; font-size: 0.75rem; color: #64748b; margin-top: 4px;">ID: ${id}</p>
                    </div>
                </div>
                <div class="card-body">
                    <p><strong>Config:</strong> <code style="font-size: 0.8rem; background: rgba(255,255,255,0.05); padding: 2px 6px; border-radius: 4px;">${configPath}</code></p>
                </div>
                <div class="card-actions">
                    <button class="btn btn-icon" title="View Details">
                        <svg viewBox="0 0 24 24" width="18" height="18"><path d="M12 4.5C7 4.5 2.73 7.61 1 12c1.73 4.39 6 7.5 11 7.5s9.27-3.11 11-7.5c-1.73-4.39-6-7.5-11-7.5zM12 17c-2.76 0-5-2.24-5-5s2.24-5 5-5 5 2.24 5 5-2.24 5-5 5zm0-8c-1.66 0-3 1.34-3 3s1.34 3 3 3 3-1.34 3-3-1.34-3-3-3z"/></svg>
                    </button>
                    <button class="btn btn-icon" title="Settings">
                        <svg viewBox="0 0 24 24" width="18" height="18"><path d="M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58a.49.49 0 0 0 .12-.61l-1.92-3.32a.488.488 0 0 0-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54a.484.484 0 0 0-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96c-.22-.08-.47 0-.59.22L2.74 8.87a.49.49 0 0 0 .12.61l2.03 1.58c-.05.3-.09.63-.09.94s.02.64.07.94l-2.03 1.58a.49.49 0 0 0-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .43-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32a.49.49 0 0 0-.12-.61l-2.03-1.58zM12 15.5c-1.93 0-3.5-1.57-3.5-3.5s1.57-3.5 3.5-3.5 3.5 1.57 3.5 3.5-1.57 3.5-3.5 3.5z"/></svg>
                    </button>
                    <button class="btn btn-icon btn-danger" title="Delete" style="margin-left: auto; border-color: rgba(239, 68, 68, 0.2); color: #ef4444;">
                         <svg viewBox="0 0 24 24" width="18" height="18"><path d="M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z"/></svg>
                    </button>
                </div>
            `;
            list.appendChild(card);

            // Trigger animation
            requestAnimationFrame(() => {
                card.style.opacity = '1';
                card.style.transform = 'translateY(0)';
            });
        });
    }

    function getTypeColor(type) {
        const types = {
            'postgres': '#336791',
            'mongodb': '#47A248',
            'redis': '#DC382D',
            'mysql': '#4479A1',
            'mariadb': '#003545',
            'sqlite': '#003B57',
            'generic': '#6366f1'
        };
        return types[type.toLowerCase()] || '#6366f1';
    }

    if (refreshBtn) {
        refreshBtn.addEventListener('click', (e) => {
            e.preventDefault();
            loadDataSources();
        });
    }

    // Initial Load
    loadDataSources();
});
