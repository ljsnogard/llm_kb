// 「连接 kb_core」对话框。
//
// 两种进入方式：
//
// - **首次运行**（配置文件不存在）：直接用预填的本机连接方式打开，让用户确认或改；
// - **手动打开**（侧边栏的连接条 / 折叠轨道上的插件图标）：列出已有连接方式，
//   选一条连，或者编辑 / 新建。
//
// 它只跟 [ConnectionController] 打交道，不直接碰生成的 FRB 代码。

import 'package:flutter/material.dart';

import '../../services/kb_client_api.dart';
import '../../state/connection_controller.dart';
import '../../theme/app_theme.dart';
import '../../theme/dsw_tokens.dart';
import '../common/dsw_controls.dart';

/// 打开「连接 kb_core」对话框。
///
/// 返回是否成功连上（用户取消或失败都是 `false`/`null`）。
Future<bool?> showConnectionDialog(
  BuildContext context,
  ConnectionController connection, {
  bool firstRun = false,
}) {
  return showDialog<bool>(
    context: context,
    barrierDismissible: !firstRun,
    builder: (BuildContext context) =>
        _ConnectionDialog(connection: connection, firstRun: firstRun),
  );
}

/// 对话框本体。
class _ConnectionDialog extends StatefulWidget {
  const _ConnectionDialog({required this.connection, required this.firstRun});

  final ConnectionController connection;
  final bool firstRun;

  @override
  State<_ConnectionDialog> createState() => _ConnectionDialogState();
}

class _ConnectionDialogState extends State<_ConnectionDialog> {
  late final TextEditingController _name;
  late final TextEditingController _kbCore;
  late final TextEditingController _runtimeDir;
  late final TextEditingController _storageDir;
  late final TextEditingController _address;

  late String _kind;
  bool _loadingSuggestion = true;
  bool _busy = false;
  String _message = '';

  @override
  void initState() {
    super.initState();
    _name = TextEditingController(text: '本机');
    _kbCore = TextEditingController();
    _runtimeDir = TextEditingController();
    _storageDir = TextEditingController();
    _address = TextEditingController(text: '127.0.0.1:8788');
    _kind = widget.connection.kindDescriptions.keys.firstOrNull ?? 'local-launch';

    // 首次运行 / 没有任何已配置项时，预填一条"启动本机 kb_core"。
    final ConnectionView? existing = widget.connection.profiles.firstOrNull;
    if (existing != null) {
      _fillFrom(existing);
      _loadingSuggestion = false;
    } else {
      _prefillSuggestion();
    }
  }

  @override
  void dispose() {
    _name.dispose();
    _kbCore.dispose();
    _runtimeDir.dispose();
    _storageDir.dispose();
    _address.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    final List<String> kinds = widget.connection.kindDescriptions.keys.toList();
    if (kinds.isNotEmpty && !kinds.contains(_kind)) {
      _kind = kinds.first;
    }

    return Dialog(
      backgroundColor: c.bgLayer2,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(20)),
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 520),
        child: SingleChildScrollView(
          padding: const EdgeInsets.all(20),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: <Widget>[
              Text(
                widget.firstRun ? '连接 kb_core' : '连接方式',
                style: DswTypography.body.copyWith(
                  fontSize: 16,
                  fontWeight: FontWeight.w600,
                  color: c.labelPrimary,
                ),
              ),
              const SizedBox(height: 6),
              Text(
                '工作区与会话由 kb_core 保管；先告诉客户端怎么找到它。',
                style: DswTypography.caption.copyWith(color: c.labelTertiary),
              ),
              if (widget.connection.configPath.isNotEmpty) ...<Widget>[
                const SizedBox(height: 6),
                Text(
                  '配置文件：${widget.connection.configPath}',
                  style: DswTypography.caption.copyWith(
                    fontSize: 11,
                    color: c.labelCaption,
                  ),
                ),
              ],
              if (widget.connection.profiles.isNotEmpty) ...<Widget>[
                const SizedBox(height: 16),
                _SectionLabel('已配置的连接方式'),
                for (final ConnectionView profile in widget.connection.profiles)
                  _ProfileRow(
                    profile: profile,
                    selected: profile.name == _name.text.trim() && _isFilled(),
                    onTap: () => setState(() => _fillFrom(profile)),
                  ),
                const SizedBox(height: 8),
              ],
              const SizedBox(height: 16),
              _SectionLabel(widget.connection.profiles.isEmpty ? '连接方式' : '新增 / 编辑'),
              const SizedBox(height: 8),
              DswLabeledField(label: '名字', controller: _name, hint: '例如：本机'),
              const SizedBox(height: 10),
              _KindPicker(
                kinds: kinds,
                value: _kind,
                describe: widget.connection.describeKind,
                onChanged: (String value) => setState(() => _kind = value),
              ),
              const SizedBox(height: 10),
              ..._fieldsFor(context),
              if (_message.isNotEmpty) ...<Widget>[
                const SizedBox(height: 12),
                Text(
                  _message,
                  style: DswTypography.caption.copyWith(
                    color: c.stateError,
                  ),
                ),
              ],
              const SizedBox(height: 20),
              Row(
                mainAxisAlignment: MainAxisAlignment.end,
                children: <Widget>[
                  DswGhostButton(
                    label: '取消',
                    onPressed: _busy ? null : () => Navigator.of(context).pop(),
                  ),
                  const SizedBox(width: 8),
                  DswPrimaryButton(
                    label: _busy ? '连接中…' : '保存并连接',
                    onPressed: _busy ? null : _saveAndConnect,
                  ),
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }

  /// 当前 `kind` 需要的输入框（不适用的不显示）。
  List<Widget> _fieldsFor(BuildContext context) {
    switch (_kind) {
      case 'local-launch':
        return <Widget>[
          DswLabeledField(
            label: 'kb-core 路径',
            controller: _kbCore,
            hint: '/usr/local/bin/kb-core',
          ),
          const SizedBox(height: 10),
          DswLabeledField(label: '运行时目录', controller: _runtimeDir),
          const SizedBox(height: 10),
          DswLabeledField(label: '存储目录', controller: _storageDir),
          if (_loadingSuggestion) ...<Widget>[
            const SizedBox(height: 6),
            Text(
              '正在读取缺省值…',
              style: DswTypography.caption.copyWith(color: context.dsw.labelCaption),
            ),
          ] else if (_kbCore.text.isEmpty) ...<Widget>[
            const SizedBox(height: 6),
            Text(
              '没猜到 kb-core 在哪，请手动填它的完整路径。',
              style: DswTypography.caption.copyWith(
                color: context.dsw.stateWarn,
              ),
            ),
          ],
        ];
      case 'local-attach':
        return <Widget>[
          DswLabeledField(label: '运行时目录', controller: _runtimeDir),
        ];
      case 'tcp':
        return <Widget>[
          DswLabeledField(
            label: '地址',
            controller: _address,
            hint: '192.168.1.5:8788',
          ),
          const SizedBox(height: 6),
          Text(
            '⚠️ 网关没有鉴权与 TLS，仅用于受信网络。',
            style: DswTypography.caption.copyWith(
              color: context.dsw.stateWarn,
            ),
          ),
        ];
      default:
        return const <Widget>[];
    }
  }

  /// 从一条连接方式填进表单。
  void _fillFrom(ConnectionView profile) {
    _kind = profile.kind;
    _name.text = profile.name;
    _kbCore.text = profile.kbCore;
    _runtimeDir.text = profile.runtimeDir;
    _storageDir.text = profile.storageDir;
    _address.text = profile.address.isEmpty ? '127.0.0.1:8788' : profile.address;
    _loadingSuggestion = false;
  }

  /// 首次运行时向原生侧要一条预填的本机连接方式。
  Future<void> _prefillSuggestion() async {
    try {
      final ConnectionView suggested = await widget.connection
          .suggestedLocalConnection(_name.text.trim().isEmpty ? '本机' : _name.text.trim());
      if (!mounted) {
        return;
      }
      setState(() {
        _fillFrom(suggested);
        if (suggested.kind.isNotEmpty) {
          _kind = suggested.kind;
        }
      });
    } catch (_) {
      // 取不到预填值也不挡路：用户手填即可。
    } finally {
      if (mounted) {
        setState(() => _loadingSuggestion = false);
      }
    }
  }

  bool _isFilled() => _name.text.trim().isNotEmpty;

  /// 组装表单里这条连接方式。
  ConnectionView _draft() {
    final String name = _name.text.trim().isEmpty ? '未命名连接' : _name.text.trim();
    switch (_kind) {
      case 'local-attach':
        return ConnectionView(
          name: name,
          kind: _kind,
          summary: widget.connection.describeKind(_kind),
          kbCore: '',
          runtimeDir: _runtimeDir.text.trim(),
          storageDir: '',
          address: '',
        );
      case 'tcp':
        return ConnectionView(
          name: name,
          kind: _kind,
          summary: widget.connection.describeKind(_kind),
          kbCore: '',
          runtimeDir: '',
          storageDir: '',
          address: _address.text.trim(),
        );
      default:
        return ConnectionView(
          name: name,
          kind: 'local-launch',
          summary: widget.connection.describeKind('local-launch'),
          kbCore: _kbCore.text.trim(),
          runtimeDir: _runtimeDir.text.trim(),
          storageDir: _storageDir.text.trim(),
          address: '',
        );
    }
  }

  /// 保存配置（合并同名项）然后连。
  Future<void> _saveAndConnect() async {
    setState(() {
      _busy = true;
      _message = '';
    });

    final ConnectionView draft = _draft();
    final List<ConnectionView> merged = <ConnectionView>[
      for (final ConnectionView profile in widget.connection.profiles)
        if (profile.name != draft.name) profile,
      draft,
    ];

    final String saveError = await widget.connection.saveConfig(
      defaultName: draft.name,
      connections: merged,
    );
    if (saveError.isNotEmpty) {
      if (mounted) {
        setState(() {
          _busy = false;
          _message = saveError;
        });
      }
      return;
    }

    final bool ok = await widget.connection.connect(draft);
    if (!mounted) {
      return;
    }
    if (ok) {
      Navigator.of(context).pop(true);
      return;
    }
    setState(() {
      _busy = false;
      _message = widget.connection.error.isEmpty
          ? '连接失败'
          : widget.connection.error;
    });
  }
}

/// 小标题。
class _SectionLabel extends StatelessWidget {
  const _SectionLabel(this.text);

  final String text;

  @override
  Widget build(BuildContext context) {
    return Text(
      text,
      style: DswTypography.caption.copyWith(
        color: context.dsw.labelTertiary,
        fontWeight: FontWeight.w500,
      ),
    );
  }
}

/// 已配置的连接方式，点一下填进表单。
class _ProfileRow extends StatelessWidget {
  const _ProfileRow({
    required this.profile,
    required this.selected,
    required this.onTap,
  });

  final ConnectionView profile;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: onTap,
      child: Container(
        margin: const EdgeInsets.only(top: 4),
        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 8),
        decoration: BoxDecoration(
          color: selected ? c.interactiveHover : Colors.transparent,
          borderRadius: BorderRadius.circular(8),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: <Widget>[
            Text(
              '${profile.name}  ·  ${profile.kind}',
              style: DswTypography.body.copyWith(
                fontSize: 13,
                color: c.labelPrimary,
              ),
            ),
            if (profile.summary.isNotEmpty)
              Text(
                profile.summary,
                style: DswTypography.caption.copyWith(color: c.labelTertiary),
              ),
          ],
        ),
      ),
    );
  }
}

/// 连接方式下拉框 + 说明。
class _KindPicker extends StatelessWidget {
  const _KindPicker({
    required this.kinds,
    required this.value,
    required this.describe,
    required this.onChanged,
  });

  final List<String> kinds;
  final String value;
  final String Function(String kind) describe;
  final ValueChanged<String> onChanged;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: <Widget>[
        Row(
          children: <Widget>[
            SizedBox(
              width: 74,
              child: Text(
                '方式',
                style: DswTypography.caption.copyWith(color: c.labelTertiary),
              ),
            ),
            Expanded(
              child: DropdownButton<String>(
                value: kinds.contains(value) ? value : null,
                isExpanded: true,
                underline: const SizedBox.shrink(),
                dropdownColor: c.bgLayer2,
                items: <DropdownMenuItem<String>>[
                  for (final String kind in kinds)
                    DropdownMenuItem<String>(value: kind, child: Text(kind)),
                ],
                onChanged: (String? next) {
                  if (next != null) {
                    onChanged(next);
                  }
                },
              ),
            ),
          ],
        ),
        if (describe(value).isNotEmpty)
          Padding(
            padding: const EdgeInsets.only(left: 74, top: 2),
            child: Text(
              describe(value),
              style: DswTypography.caption.copyWith(
                fontSize: 11,
                color: c.labelCaption,
              ),
            ),
          ),
      ],
    );
  }
}
