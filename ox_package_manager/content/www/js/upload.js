document.addEventListener('DOMContentLoaded', () => {
    const dropZone = document.getElementById('drop-zone');
    const fileInput = document.getElementById('file-input');
    const browseBtn = document.getElementById('browse-btn');
    const progressContainer = document.getElementById('progress-container');
    const progressBar = document.getElementById('progress-fill');
    const statusText = document.getElementById('status-text');
    const stagedList = document.getElementById('staged-list');

    // Initialize Status Badge
    if (typeof initGlobalStatus === 'function') {
        initGlobalStatus();
    }

    // Create Modal Element
    const modal = document.createElement('dialog');
    modal.innerHTML = `
        <div class="modal-header">
            <h3 class="modal-title">Package Details</h3>
            <button class="modal-close">&times;</button>
        </div>
        <div id="modal-content"></div>
        <div style="text-align: right; margin-top: 1.5rem;">
            <button class="btn" id="modal-ok-btn" style="margin-top:0; padding: 0.5rem 1.5rem;">Close</button>
        </div>
    `;
    document.body.appendChild(modal);

    const closeModal = () => modal.close();
    modal.querySelector('.modal-close').addEventListener('click', closeModal);
    modal.querySelector('#modal-ok-btn').addEventListener('click', closeModal);
    modal.addEventListener('click', (e) => {
        if (e.target === modal) closeModal();
    });

    // Load initial lists
    loadStagedPackages();
    loadInstalledPackages();

    // Drag & Drop Events
    ['dragenter', 'dragover', 'dragleave', 'drop'].forEach(eventName => {
        dropZone.addEventListener(eventName, preventDefaults, false);
    });

    function preventDefaults(e) {
        e.preventDefault();
        e.stopPropagation();
    }

    ['dragenter', 'dragover'].forEach(eventName => {
        dropZone.addEventListener(eventName, highlight, false);
    });

    ['dragleave', 'drop'].forEach(eventName => {
        dropZone.addEventListener(eventName, unhighlight, false);
    });

    function highlight(e) {
        dropZone.classList.add('drag-over');
    }

    function unhighlight(e) {
        dropZone.classList.remove('drag-over');
    }

    dropZone.addEventListener('drop', handleDrop, false);

    function handleDrop(e) {
        const dt = e.dataTransfer;
        const files = dt.files;
        handleFiles(files);
    }

    // Button Click
    browseBtn.addEventListener('click', () => {
        fileInput.click();
    });

    fileInput.addEventListener('change', function () {
        handleFiles(this.files);
    });

    function handleFiles(files) {
        ([...files]).forEach(uploadFile);
    }

    const cancelBtn = document.getElementById('cancel-btn');
    const ALLOWED_EXTENSIONS = ['.tar.gz', '.tgz', '.zip', '.tar.bz2', '.tbz2'];

    function uploadFile(file) {
        // Validate Extension
        const lowercaseName = file.name.toLowerCase();
        const isValid = ALLOWED_EXTENSIONS.some(ext => lowercaseName.endsWith(ext));

        if (!isValid) {
            statusText.textContent = `Error: Invalid file type. Allowed: ${ALLOWED_EXTENSIONS.join(', ')}`;
            statusText.style.color = 'var(--error)';
            progressContainer.style.display = 'block';
            progressBar.style.width = '0%';
            console.error(`Invalid extension for file: ${file.name}`);
            return;
        }

        // Prepare UI
        progressContainer.style.display = 'block';
        progressBar.style.width = '0%';
        statusText.textContent = `Uploading ${file.name}...`;
        statusText.style.color = 'var(--text-primary)'; // Reset color

        cancelBtn.style.display = 'block';

        const formData = new FormData();
        formData.append('package', file);

        const xhr = new XMLHttpRequest();
        xhr.open('POST', '/packages/upload/', true);

        cancelBtn.onclick = () => {
            xhr.abort();
        };

        xhr.upload.onprogress = function (e) {
            if (e.lengthComputable) {
                const percentComplete = (e.loaded / e.total) * 100;
                progressBar.style.width = percentComplete + '%';
                statusText.textContent = `Uploading ${file.name}: ${Math.round(percentComplete)}%`;
            }
        };

        xhr.onload = function () {
            cancelBtn.style.display = 'none';
            let response;
            try {
                response = JSON.parse(xhr.responseText);
            } catch (e) {
                // Fallback for non-JSON response
                response = { result: 'error', message: xhr.responseText };
            }

            if (xhr.status === 200 && response.result === 'success') {
                progressBar.style.width = '100%';
                statusText.textContent = `Upload Complete: ${file.name}`;
                statusText.style.color = 'var(--success)';
                loadStagedPackages(); // Refresh listing
            } else {
                const errorMsg = response.message || 'Unknown error occurred';
                statusText.textContent = `Error: ${errorMsg}`;
                statusText.style.color = 'var(--error)';
            }
        };

        xhr.onerror = function () {
            cancelBtn.style.display = 'none';
            statusText.textContent = 'Upload Failed (Network Error)';
            statusText.style.color = 'var(--error)';
        };

        xhr.onabort = function () {
            cancelBtn.style.display = 'none';
            statusText.textContent = 'Upload Cancelled';
            statusText.style.color = 'var(--warning)';
            progressBar.style.width = '0%';
        };

        xhr.send(formData);
    }

    function formatBytes(bytes, decimals = 2) {
        if (!+bytes) return '0 Bytes';
        const k = 1024;
        const dm = decimals < 0 ? 0 : decimals;
        const sizes = ['Bytes', 'KiB', 'MiB', 'GiB', 'TiB'];
        const i = Math.floor(Math.log(bytes) / Math.log(k));
        return `${parseFloat((bytes / Math.pow(k, i)).toFixed(dm))} ${sizes[i]}`;
    }

    function loadStagedPackages() {
        stagedList.innerHTML = `<h4 style="margin-bottom: 1rem; color: var(--text-secondary); text-transform: uppercase; font-size: 0.8rem;">Staged Packages</h4>`;
        const loading = document.createElement('div');
        loading.textContent = "Loading...";
        loading.style.color = "var(--text-secondary)";
        loading.style.textAlign = "center";
        stagedList.appendChild(loading);

        fetch('/packages/list/')
            .then(res => res.json())
            .then(data => {
                stagedList.removeChild(loading);
                if (data.result === 'success' && data.packages) {
                    if (data.packages.length === 0) {
                        const empty = document.createElement('div');
                        empty.textContent = "No packages staged.";
                        empty.style.color = "var(--text-secondary)";
                        empty.style.fontStyle = "italic";
                        empty.style.textAlign = "center";
                        stagedList.appendChild(empty);
                    } else {
                        data.packages.forEach(pkg => addStagedItem(pkg));
                    }
                } else {
                    console.error("Failed to load packages:", data);
                }
            })
            .catch(err => {
                loading.textContent = "Failed to load staged packages.";
                console.error(err);
            });
    }

    function addStagedItem(pkg) {
        const div = document.createElement('div');
        div.className = 'pkg-item';
        div.innerHTML = `
            <div style="display: flex; align-items: center; gap: 0.5rem;">
                <span class="pkg-name">${pkg.filename}</span>
                <button class="btn-icon info-btn" title="View Details">ℹ️</button>
            </div>
            <div style="display: flex; align-items: center; gap: 1rem;">
                <div style="text-align: right; font-size: 0.8rem; color: var(--text-secondary);">
                    <div>${pkg.version ? 'v' + pkg.version : ''}</div>
                    <div>${formatBytes(pkg.size)}</div>
                </div>
                <button class="btn btn-install install-btn" style="margin:0;">Install</button>
            </div>
        `;
        div.querySelector('.info-btn').addEventListener('click', () => showPackageInfo(pkg));
        div.querySelector('.install-btn').addEventListener('click', () => installPackage(pkg.filename));
        stagedList.appendChild(div);
    }

    window.installPackage = async function (filename) {
        if (!confirm(`Are you sure you want to install ${filename}?`)) return;
        try {
            const response = await fetch('/packages/install/', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ filename: filename })
            });
            const result = await response.json();
            if (result.result === 'success') {
                alert('Package installed successfully!');
                loadStagedPackages();
                loadInstalledPackages();
            } else {
                alert('Installation failed: ' + (result.message || 'Unknown error'));
            }
        } catch (error) {
            console.error('Install error:', error);
            alert('Installation failed due to network or server error.');
        }
    };

    function loadInstalledPackages() {
        const list = document.getElementById('installed-list');
        list.innerHTML = '';
        const loading = document.createElement('div');
        loading.textContent = "Loading installed packages...";
        loading.style.textAlign = 'center';
        loading.style.color = 'var(--text-secondary)';
        list.appendChild(loading);

        fetch('/packages/list/installed')
            .then(res => res.json())
            .then(data => {
                list.removeChild(loading);
                if (data.result === 'success' && data.packages) {
                    if (data.packages.length === 0) {
                        list.innerHTML = '<div style="text-align:center; color:var(--text-secondary); font-style:italic;">No packages installed.</div>';
                    } else {
                        // Build checks for package hierarchy
                        // 1. Identify which packages handle other types
                        const packages = data.packages;
                        const handlerMap = {}; // type -> module_package_name
                        packages.forEach(pkg => {
                            if (pkg.installer_handlers && typeof pkg.installer_handlers === 'object') {
                                for (const type in pkg.installer_handlers) {
                                    handlerMap[type] = pkg.name;
                                }
                            }
                        });

                        // 2. Identify top-level packages (Modules only)
                        const topLevel = packages.filter(pkg => pkg.package_type === 'module');

                        topLevel.forEach(pkg => {
                            try {
                                renderInstalledItem(pkg, packages, list);
                            } catch (renderErr) {
                                console.error('Error rendering package:', pkg.name, renderErr);
                                const errDiv = document.createElement('div');
                                errDiv.style.color = 'red';
                                errDiv.textContent = `Error rendering ${pkg.name}: ${renderErr.message}`;
                                list.appendChild(errDiv);
                            }
                        });
                    }
                } else {
                    list.textContent = "Failed to load installed packages: " + (data.message || "Unknown error");
                }
            })
            .catch(e => {
                list.textContent = "Error loading installed packages: " + e.message;
                console.error(e);
            });
    }
    // Expose to window for HTML onclick handlers
    window.loadInstalledPackages = loadInstalledPackages;
    window.loadStagedPackages = loadStagedPackages;

    function renderInstalledItem(pkg, allPackages, container) {
        const div = document.createElement('div');
        div.className = 'pkg-item';

        // Calculate Dependents (Reverse Dependencies)
        // Find all packages p where p.dependencies includes pkg.name
        const dependents = allPackages.filter(p => p.dependencies && Array.isArray(p.dependencies) && p.dependencies.includes(pkg.name));

        const hasDependents = dependents.length > 0;
        const showExpand = hasDependents || pkg.description;

        let html = `
            <div style="display: flex; flex-direction: column; width: 100%;">
                <div style="display: flex; align-items: center; justify-content: space-between; width: 100%;">
                    <div style="display: flex; align-items: center; gap: 0.5rem;">
                        ${showExpand ? `<span class="toggle-sub" style="cursor:pointer; width:1.5rem; text-align:center;">▶</span>` : `<span style="width:1.5rem;"></span>`}
                        <span class="pkg-name">${pkg.name}</span>
                        <span class="badge badge-neutral" style="font-size: 0.7rem; opacity: 0.7;">${pkg.package_type || 'unknown'}</span>
                    </div>
                    <div style="display: flex; align-items: center; gap: 1rem;">
                        <div style="text-align: right; font-size: 0.8rem; color: var(--text-secondary);">
                            <div>${pkg.version ? 'v' + pkg.version : ''}</div>
                        </div>
                        <button class="btn btn-delete uninstall-btn" style="margin:0; background: var(--error); color: white;">Uninstall</button>
                    </div>
                </div>
        `;

        if (showExpand) {
            html += `<div class="sub-packages" style="display:none; padding-left: 2rem; margin-top: 0.5rem; border-left: 1px solid var(--border-color);"></div>`;
        }

        html += `</div>`;
        div.innerHTML = html;

        const uninstallBtn = div.querySelector('.uninstall-btn');
        if (uninstallBtn) {
            uninstallBtn.addEventListener('click', (e) => {
                e.stopPropagation();
                confirmUninstall(pkg, allPackages);
            });
        }

        if (showExpand) {
            const toggle = div.querySelector('.toggle-sub');
            const subContainer = div.querySelector('.sub-packages');

            if (toggle && subContainer) {
                toggle.addEventListener('click', (e) => {
                    e.stopPropagation();
                    if (subContainer.style.display === 'none') {
                        subContainer.style.display = 'block';
                        toggle.textContent = '▼';

                        // Populate content if empty
                        if (subContainer.innerHTML === '') {
                            // 1. Module Info Block
                            const infoDiv = document.createElement('div');
                            infoDiv.className = 'pkg-details';
                            infoDiv.style.marginBottom = '1rem';
                            infoDiv.style.padding = '0.5rem';
                            infoDiv.style.background = 'rgba(255, 255, 255, 0.03)';
                            infoDiv.style.borderRadius = '4px';
                            infoDiv.style.fontSize = '0.9rem';

                            const depList = (pkg.dependencies && pkg.dependencies.length) ? pkg.dependencies.join(', ') : 'None';
                            const handlers = (pkg.installer_handlers && Object.keys(pkg.installer_handlers).length > 0) ? JSON.stringify(pkg.installer_handlers) : 'None';

                            infoDiv.innerHTML = `
                                <div style="font-weight: bold; margin-bottom: 0.25rem; color: var(--accent-color);">Package Information</div>
                                <div style="display: grid; grid-template-columns: auto 1fr; gap: 0.5rem 1rem;">
                                    <span style="color: var(--text-secondary);">Description:</span> <span>${pkg.description || 'N/A'}</span>
                                    <span style="color: var(--text-secondary);">Type:</span> <span>${pkg.package_type}</span>
                                    <span style="color: var(--text-secondary);">Dependencies:</span> <span>${depList}</span>
                                    <span style="color: var(--text-secondary);">Handlers:</span> <span>${handlers}</span>
                                    <span style="color: var(--text-secondary);">Dependents:</span> <span>${dependents.length}</span>
                                </div>
                             `;
                            subContainer.appendChild(infoDiv);

                            // 2. Dependents List
                            if (hasDependents) {
                                const depsHeader = document.createElement('div');
                                depsHeader.textContent = "Modules dependent on this package";
                                depsHeader.style.fontSize = '0.8rem';
                                depsHeader.style.textTransform = 'uppercase';
                                depsHeader.style.color = 'var(--text-secondary)';
                                depsHeader.style.marginBottom = '0.5rem';
                                depsHeader.style.marginTop = '0.5rem';
                                subContainer.appendChild(depsHeader);

                                dependents.forEach(child => renderInstalledItem(child, allPackages, subContainer));
                            }
                        }
                    } else {
                        subContainer.style.display = 'none';
                        toggle.textContent = '▶';
                    }
                });
            }
        }

        container.appendChild(div);
    }

    function confirmUninstall(pkg, allPackages) {
        // Step 1: Initial Verification
        if (!confirm(`Are you sure you want to uninstall '${pkg.name}'?`)) {
            return;
        }

        // Step 2: Check for Dependents (Recursive)
        const dependents = getRecursiveDependents(pkg.name, allPackages);

        if (dependents.length > 0) {
            // Found dependents
            const depNames = dependents.map(p => p.name).join(', ');

            // Step 3: Warning
            if (!confirm(`WARNING: The following packages depend on '${pkg.name}' and will ALSO be uninstalled:\n\n${depNames}\n\nDo you want to continue?`)) {
                return;
            }

            // Step 4: Final List & Confirmation
            const allToRemove = [...dependents, pkg]; // Dependents first, then target
            const listStr = allToRemove.map(p => `- ${p.name}`).join('\n');

            if (!confirm(`The following packages will be uninstalled:\n${listStr}\n\nThis action cannot be undone. Proceed?`)) {
                return;
            }

            // Execute Sequential Uninstall (Dependents first)
            performSequentialUninstall(allToRemove);

        } else {
            // No dependents, just uninstall target
            performUninstall(pkg.name);
        }
    }

    function getRecursiveDependents(targetName, allPackages, visited = new Set()) {
        const directDependents = allPackages.filter(p =>
            p.dependencies &&
            Array.isArray(p.dependencies) &&
            p.dependencies.includes(targetName) &&
            !visited.has(p.name)
        );

        let allDependents = [...directDependents];
        directDependents.forEach(dep => {
            visited.add(dep.name);
            const subDependents = getRecursiveDependents(dep.name, allPackages, visited);
            // Add only new ones
            subDependents.forEach(sd => {
                if (!allDependents.find(ad => ad.name === sd.name)) {
                    allDependents.push(sd);
                }
            });
        });

        return allDependents;
    }

    function performSequentialUninstall(packages) {
        // packages is list of objects. We want to uninstall them in order (0 to N).
        // Since we collected dependents first, this is the correct order (Leaves -> Root).

        if (packages.length === 0) {
            alert('All selected packages have been uninstalled.');
            loadInstalledPackages(); // Refresh UI
            loadStagedPackages();
            return;
        }

        const current = packages[0];
        const remaining = packages.slice(1);

        console.log(`Uninstalling ${current.name}...`);

        fetch('/packages/uninstall', {
            method: 'POST',
            body: JSON.stringify({ package: current.name }),
            headers: { 'Content-Type': 'application/json' }
        })
            .then(res => res.json())
            .then(data => {
                if (data.result !== 'success') {
                    alert(`Failed to uninstall ${current.name}: ${data.message}\nStopping sequence.`);
                    loadInstalledPackages();
                } else {
                    performSequentialUninstall(remaining);
                }
            })
            .catch(err => {
                alert(`Error uninstalling ${current.name}: ${err.message}`);
                loadInstalledPackages();
            });
    }

    function performUninstall(pkgName) {
        fetch('/packages/uninstall', {
            method: 'POST',
            body: JSON.stringify({ package: pkgName }),
            headers: { 'Content-Type': 'application/json' }
        })
            .then(res => res.json())
            .then(data => {
                if (data.result === 'success') {
                    alert("Package uninstalled successfully.");
                    loadInstalledPackages();
                    loadStagedPackages();
                } else {
                    alert("Failed to uninstall package: " + (data.message || "Unknown error"));
                }
            })
            .catch(e => {
                alert("Error uninstalling package: " + e.message);
            });
    }

    window.showPackageInfo = function (pkg) {
        fetch(`/packages/installed/package?name=${pkg.name}`)
            .then(res => {
                if (res.ok) return res.json();
                return pkg;
            })
            .then(details => {
                const content = document.getElementById('modal-content');
                content.innerHTML = `
                    <div class="key-value"><label>Name:</label><div>${details.name}</div></div>
                    <div class="key-value"><label>Version:</label><div>${details.version}</div></div>
                    <div class="key-value"><label>Type:</label><div>${details.package_type}</div></div>
                    <div class="key-value"><label>Description:</label><div>${details.description || 'N/A'}</div></div>
                    <hr style="border-color: var(--border-color); opacity: 0.3; margin: 1rem 0;">
                    <div class="key-value"><label>Dependencies:</label><div>${(details.dependencies || []).join(', ') || 'None'}</div></div>
                    <div class="key-value"><label>Handlers:</label><div>${JSON.stringify(details.installer_handlers || {})}</div></div>
                 `;
                document.querySelector('dialog').showModal();
            })
            .catch(e => {
                console.error(e);
                const content = document.getElementById('modal-content');
                content.innerHTML = `
                    <div class="key-value"><label>Name:</label><div>${pkg.name}</div></div>
                    <div class="key-value"><label>Version:</label><div>${pkg.version}</div></div>
                    <div class="key-value"><label>Type:</label><div>${pkg.package_type}</div></div>
                 `;
                document.querySelector('dialog').showModal();
            });
    };
});
