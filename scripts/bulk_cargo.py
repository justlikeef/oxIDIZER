import os
import re

plugins = [
    "ox_webservice_errorhandler_jinja2", "ox_webservice_errorhandler_json", "ox_webservice_template_jinja2",
    "ox_webservice_rewrite", "ox_webservice_redirect", "ox_webservice_stream", "ox_webservice_status",
    "ox_webservice_restore_ip", "ox_webservice_wsgi", "ox_webservice_forwarded_for", "ox_webservice_vary_header",
    "ox_webservice_test_utils", "ox_package_manager", "ox_forms/ox_forms_server", "ox_forms/ox_forms_api",
    "ox_forms/ox_forms_std_renderers", "ox_forms/ox_forms_client", "ox_auth_ip", "ox_data_broker",
    "ox_data_object/ox_data_object_manager",
    "ox_data_object/ox_data_object_dictionary_manager", "ox_persistence_datasource_manager",
    "ox_persistence_driver_installer", "ox_persistence_driver_manager", "ox_persistence/ox_persistence_dictionary_manager",
    "ox_content", "ox_server_info"
]

for p in plugins:
    cargo_path = f"/var/repos/oxIDIZER/{p}/Cargo.toml"
    if not os.path.exists(cargo_path):
        print(f"Skipping {cargo_path}")
        continue
    with open(cargo_path, 'r') as f:
        content = f.read()

    # Remove bumpalo
    content = re.sub(r'(?m)^bumpalo\s*=.*$\n?', '', content)
    
    # Replace ox_pipeline_plugin with ox_workflow_abi
    # Need to compute relative path to oxWorkflow based on depth
    depth = len(p.split('/'))
    rel_path = "../" * depth + "../oxWorkflow/ox_workflow_abi"
    replacement = f'ox_workflow_abi = {{ path = "{rel_path}" }}'
    content = re.sub(r'(?m)^ox_pipeline_plugin\s*=.*$', replacement, content)

    # Note: If ox_workflow_abi already exists, don't duplicate
    if "ox_workflow_abi" not in content and replacement not in content:
        # Fallback if no ox_pipeline_plugin line existed
        pass

    with open(cargo_path, 'w') as f:
        f.write(content)
        print(f"Updated {cargo_path}")

