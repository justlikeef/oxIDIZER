import re

with open("DESIGN.md", "r") as f:
    text = f.read()

text = text.replace("mTLS, admin role cert", "mTLS, operator cert")
text = text.replace("mTLS, approver role cert — distinct from admin", "mTLS, operator cert — not submitter")
text = text.replace("mTLS (admin role cert)", "mTLS (operator cert)")
text = text.replace("Admin role certificate", "Operator role certificate ")
text = text.replace("Admin operator", "Authorized operator")
text = text.replace("mTLS auth to broker admin API + manifest deploy", "mTLS auth to broker APIs + manifest deploy")
text = re.sub(r'\|\s*Approver role certificate.*?\|\n', '', text)
text = text.replace("admin role certificate required", "operator certificate required")
text = text.replace("approver role certificate required — distinct from admin", "operator certificate required — must not be submitter")

text = re.sub(
    r'The admin role cert and approver role cert must be issued to.*?ensuring separation of duties\.',
    'Any authorized operator can submit or approve a template. However, the broker structurally enforces separation of duties: the `POST /broker/pending/{template_id}/approve` handler ensures that the `operator_id` (extracted from the mTLS certificate) granting the approval is **strictly not equal** to the `submitted_by` identity recorded when the template was created. This ensures no single user can authorize their own manifest.',
    text,
    flags=re.DOTALL
)

with open("DESIGN.md", "w") as f:
    f.write(text)

with open("IMPLEMENTATION_PLAN.md", "r") as f:
    impl = f.read()

impl = impl.replace("admin cert", "operator cert")
impl = impl.replace("approver cert", "operator cert")
impl = impl.replace("mTLS (admin role cert)", "mTLS (operator cert)")

with open("IMPLEMENTATION_PLAN.md", "w") as f:
    f.write(impl)
