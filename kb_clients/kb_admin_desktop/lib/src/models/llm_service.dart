// LLM 服务配置。
//
// 与 `kb_svc_salvo` 的 `settings::LlmServiceConfig` 以及
// `GET /api/settings` 的应答字段一一对应（见 `kb_svc_salvo/src/web.rs`），
// 便于后续把本地配置与服务端配置对齐。

import 'package:flutter/foundation.dart';

/// `kb_svc_salvo` 用来遮蔽 API key 的占位串（U+2022 ×8）。
///
/// 界面回填时原样提交这个值，服务端会保留原有 key 而不是把占位串写进配置。
const String kMaskedApiKey = '••••••••';

/// 一个可用的 LLM 服务。
@immutable
class LlmService {
  /// 构造一个服务配置。
  const LlmService({
    required this.id,
    required this.provider,
    required this.model,
    this.baseUrl = '',
    this.apiKey = '',
  });

  /// 服务标识（用户可见的名字）。
  final String id;

  /// provider 标识，例如 `deepseek`。
  final String provider;

  /// 模型名，例如 `deepseek-chat`。
  final String model;

  /// API base URL，可为空。
  final String baseUrl;

  /// API key。
  final String apiKey;

  /// 是否已经填了 API key。
  bool get hasApiKey => apiKey.trim().isNotEmpty;

  /// 界面展示用的「provider · model」摘要。
  String get summary =>
      baseUrl.trim().isEmpty ? '$provider · $model' : '$provider · $model · $baseUrl';

  /// 返回一份替换了若干字段的副本。
  LlmService copyWith({
    String? id,
    String? provider,
    String? model,
    String? baseUrl,
    String? apiKey,
  }) {
    return LlmService(
      id: id ?? this.id,
      provider: provider ?? this.provider,
      model: model ?? this.model,
      baseUrl: baseUrl ?? this.baseUrl,
      apiKey: apiKey ?? this.apiKey,
    );
  }

  /// 从 JSON 还原。
  factory LlmService.fromJson(Map<String, dynamic> json) => LlmService(
    id: json['id'] as String? ?? '',
    provider: json['provider'] as String? ?? '',
    model: json['model'] as String? ?? '',
    baseUrl: json['base_url'] as String? ?? '',
    apiKey: json['api_key'] as String? ?? '',
  );

  /// 序列化为 JSON。
  Map<String, dynamic> toJson() => <String, dynamic>{
    'id': id,
    'provider': provider,
    'model': model,
    'base_url': baseUrl,
    'api_key': apiKey,
  };

  @override
  bool operator ==(Object other) =>
      other is LlmService &&
      other.id == id &&
      other.provider == provider &&
      other.model == model &&
      other.baseUrl == baseUrl &&
      other.apiKey == apiKey;

  @override
  int get hashCode => Object.hash(id, provider, model, baseUrl, apiKey);
}
