/** Keep the token list in sync with `FORBIDDEN` in crates/late-core/src/stage.rs. */
export const STAGE_SECRET_TOKENS = [
  "password",
  "passwd",
  "ansible_ssh_pass",
  "ansible_become_pass",
  "ansible_sudo_pass",
  "community_string",
  "snmpv3",
  "private_key",
  "begin rsa",
  "begin openssh",
  "aws_secret",
  "client_secret",
  "api_key",
  "sshpass",
] as const;

export const STAGE_FORMATS = ["cli", "ansible", "netmiko", "salt", "chef"] as const;
export type StageFormat = (typeof STAGE_FORMATS)[number];

export function coerceStageFormat(v: unknown, fallback: StageFormat = "cli"): StageFormat {
  const s = String(v ?? "").trim().toLowerCase();
  return (STAGE_FORMATS as readonly string[]).includes(s) ? (s as StageFormat) : fallback;
}

/** Monaco language id for a staging format. */
export function stageMonacoLanguage(format: StageFormat): string {
  switch (format) {
    case "ansible":
    case "salt":
      return "yaml";
    case "netmiko":
      return "python";
    case "chef":
      return "ruby";
    default:
      return "plaintext";
  }
}

export function stageLanguageLabel(format: StageFormat): string {
  switch (format) {
    case "cli":
      return "CLI";
    case "ansible":
      return "YAML · Ansible";
    case "netmiko":
      return "Python · Netmiko";
    case "salt":
      return "YAML · Salt";
    case "chef":
      return "Ruby · Chef";
  }
}

export type StageSecretHit = { line: number; text: string };

export function stageSecretHits(body: string): StageSecretHit[] {
  const hits: StageSecretHit[] = [];
  const lines = body.split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    const lower = lines[i].toLowerCase();
    if (STAGE_SECRET_TOKENS.some((k) => lower.includes(k))) {
      hits.push({
        line: i + 1,
        text: "This line looks like a credential. Staging will refuse to save passwords, keys, or community strings.",
      });
    }
  }
  return hits;
}

export type StageSuggest = {
  label: string;
  insertText: string;
  detail: string;
  documentation: string;
};

export function stageSuggestions(format: StageFormat): StageSuggest[] {
  switch (format) {
    case "ansible":
      return ANSIBLE;
    case "netmiko":
      return NETMIKO;
    case "salt":
      return SALT;
    case "chef":
      return CHEF;
    default:
      return CLI;
  }
}

const ANSIBLE: StageSuggest[] = [
  {
    label: "playbook",
    insertText:
      "---\n- name: ${1:change}\n  hosts: late_targets\n  gather_facts: false\n  vars:\n    ansible_connection: network_cli\n  tasks:\n    - name: ${2:apply}\n      ansible.netcommon.cli_config:\n        config: |\n          ${3:# vendor syntax — no login vars}\n",
    detail: "Playbook skeleton",
    documentation: "Uses Late inventory host late_targets. Do not add login variables — Late injects auth at Push.",
  },
  {
    label: "hosts: late_targets",
    insertText: "hosts: late_targets",
    detail: "Inventory group",
    documentation: "PATH Push builds this group from the SSH device you pick in Staging.",
  },
  {
    label: "ansible.netcommon.cli_config",
    insertText:
      "- name: ${1:apply config}\n  ansible.netcommon.cli_config:\n    config: |\n      ${2:# vendor syntax}\n",
    detail: "Network CLI task",
    documentation: "Vendor-neutral config push. Set Vendor/OS on the device so ansible_network_os is correct — Late will not assume Cisco IOS.",
  },
  {
    label: "ansible.builtin.debug",
    insertText: "- name: ${1:review}\n  ansible.builtin.debug:\n    msg: ${2:message}\n",
    detail: "Linux / review task",
    documentation: "Useful on Linux inventory. Replace with the real module before Push.",
  },
  {
    label: "ansible_connection: network_cli",
    insertText: "ansible_connection: network_cli",
    detail: "Network connection",
    documentation: "For network OS devices. Linux hosts should gather_facts and use ansible.builtin modules instead.",
  },
  {
    label: "gather_facts: false",
    insertText: "gather_facts: false",
    detail: "Skip facts",
    documentation: "Network devices usually cannot gather facts.",
  },
];

const NETMIKO: StageSuggest[] = [
  {
    label: "ConnectHandler",
    insertText:
      "from netmiko import ConnectHandler\nimport os\n\nconn = ConnectHandler(\n    device_type=os.environ.get(\"LATE_DEVICE_TYPE\", DEVICE_TYPE),\n    host=os.environ[\"LATE_HOST\"],\n    username=os.environ[\"LATE_USER\"],\n    key_file=os.environ.get(\"LATE_KEY_FILE\") or None,\n    port=int(os.environ.get(\"LATE_PORT\", \"22\")),\n    use_keys=True,\n)\n${1:conn.send_config_set([])}\nconn.disconnect()\n",
    detail: "Open session from Late env",
    documentation: "Push sets LATE_HOST, LATE_USER, LATE_KEY_FILE, LATE_PORT, LATE_DEVICE_TYPE. Do not hard-code credentials.",
  },
  {
    label: "os.environ LATE_*",
    insertText:
      "host = os.environ[\"LATE_HOST\"]\nuser = os.environ[\"LATE_USER\"]\nkey = os.environ.get(\"LATE_KEY_FILE\")\nport = int(os.environ.get(\"LATE_PORT\", \"22\"))\ndevice_type = os.environ.get(\"LATE_DEVICE_TYPE\", DEVICE_TYPE)\n",
    detail: "Runtime auth env",
    documentation: "Late injects these on your computer at Push. Leave credentials out of the file.",
  },
  {
    label: "send_config_set",
    insertText: "conn.send_config_set([\n    ${1:# vendor syntax},\n])\n",
    detail: "Config lines",
    documentation: "List of configuration commands for the device OS.",
  },
  {
    label: "send_command",
    insertText: "output = conn.send_command(${1:\"show version\"})\nprint(output)\n",
    detail: "Show / exec",
    documentation: "Read-only command. Review output before you add writes.",
  },
];

const SALT: StageSuggest[] = [
  {
    label: "state id",
    insertText:
      "${1:late_change}:\n  ${2:cmd.run}:\n    - name: ${3:echo review}\n",
    detail: "SLS id",
    documentation: "Salt state file. Do not put credentials here. Push runs salt-call --local on your computer.",
  },
  {
    label: "cmd.run",
    insertText: "cmd.run:\n  - name: ${1:command}\n",
    detail: "Run a command",
    documentation: "Replace with the real state after Vendor/OS is set.",
  },
  {
    label: "file.managed",
    insertText: "file.managed:\n  - name: ${1:/tmp/review}\n    - contents: |\n        ${2:text}\n",
    detail: "Manage a file",
    documentation: "Local file on the machine that runs salt-call, not a login secret.",
  },
  {
    label: "pkg.installed",
    insertText: "pkg.installed:\n  - name: ${1:package}\n",
    detail: "Install package",
    documentation: "Linux-oriented. Network OS devices usually need napalm/cmd states instead.",
  },
];

const CHEF: StageSuggest[] = [
  {
    label: "log",
    insertText: "log '${1:late-review}' do\n  message ${2:'review'}\n  level :info\nend\n",
    detail: "Log resource",
    documentation: "Review-only. Replace with real resources after Vendor/OS is set. chef-apply runs on your computer.",
  },
  {
    label: "package",
    insertText: "package '${1:name}' do\n  action :install\nend\n",
    detail: "Package resource",
    documentation: "Linux-oriented. Do not add login credentials.",
  },
  {
    label: "service",
    insertText: "service '${1:name}' do\n  action [:enable, :start]\nend\n",
    detail: "Service resource",
    documentation: "Linux-oriented.",
  },
  {
    label: "file",
    insertText: "file '${1:/tmp/review}' do\n  content ${2:'text'}\n  mode '0644'\nend\n",
    detail: "File resource",
    documentation: "Writes on the machine running chef-apply.",
  },
];

const CLI: StageSuggest[] = [
  {
    label: "intent comment",
    insertText: "! intent: ${1:change}\n! review before Push — Late will not write memory or reboot for you\n",
    detail: "Review banner",
    documentation: "CLI Push types these lines into the open SSH/serial session you pick.",
  },
  {
    label: "configure terminal",
    insertText: "configure terminal\n${1:! vendor syntax}\nend\n",
    detail: "IOS-like config",
    documentation: "Only if this device's Vendor/OS uses configure terminal (IOS, EOS, AOS-CX, NX-OS). Do not guess Cisco on Generic.",
  },
  {
    label: "configure (Junos)",
    insertText: "configure\n${1:# set/delete lines}\n# commit only after you review\n",
    detail: "Junos config",
    documentation: "Only for Junos inventory. Leave commit for after review.",
  },
];
