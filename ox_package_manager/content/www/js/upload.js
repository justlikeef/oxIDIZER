document.addEventListener('DOMContentLoaded', () => {
    const dropZone = document.getElementById('drop-zone');
    const fileInput = document.getElementById('file-input');
    const browseBtn = document.getElementById('browse-btn');
    const progressContainer = document.getElementById('progress-container');
    const progressBar = document.getElementById('progress-fill');
    const statusText = document.getElementById('status-text');
    const stagedList = document.getElementById('staged-list');

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

    // Load initial list
    loadStagedPackages();

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
        stagedList.innerHTML = `
            <h4 style="margin-bottom: 1rem; color: var(--text-secondary); text-transform: uppercase; font-size: 0.8rem;">
            Staged Packages</h4>`;

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
                loading.textContent = "Failed to load packages.";
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

    async function installPackage(filename) {
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
            } else {
                alert('Installation failed: ' + (result.message || 'Unknown error'));
            }
        } catch (error) {
            console.error('Install error:', error);
            alert('Installation failed due to network or server error.');
        }
    }

    window.showPackageInfo = showPackageInfo;
    window.installPackage = installPackage;

    function showPackageInfo(pkg) {
        const content = document.getElementById('modal-content');
        content.innerHTML = `
            <div class="key-value">
                <label>Name:</label>
                <div>${pkg.name || 'N/A'}</div>
            </div>
            <div class="key-value">
                <label>Version:</label>
                <div>${pkg.version || 'N/A'}</div>
            </div>
            <div class="key-value">
                <label>File:</label>
                <div>${pkg.filename}</div>
            </div>
            <div class="key-value">
                <label>Size:</label>
                <div>${formatBytes(pkg.size)}</div>
            </div>
            <div class="key-value">
                <label>Description:</label>
                <div>${pkg.description || 'No description provided.'}</div>
            </div>
        `;
        modal.showModal();
    }
});
