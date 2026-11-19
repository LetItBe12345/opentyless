# Qwen-Audio ASR 与个人词表

短录音可使用 `qwen-audio-3.1-asr-flash`。设置 `ASR_MODEL` 后，程序使用 DashScope 原生多模态接口；旧 `qwen3-asr-flash` 继续使用 OpenAI 兼容接口。`ASR_API_URL` 非空时始终优先采用该地址。

```dotenv
ASR_MODEL=qwen-audio-3.1-asr-flash
ASR_API_URL=
ASR_VOCABULARY_FILE=/absolute/path/vocabulary.json
ASR_CONTEXT=软件开发、云部署、容器、CI/CD、科学计算，中英混合口述。
```

词表为 JSON 对象，例如 `{"Codex":4,"Docker Compose":4,"持久化":3}`。最多 2000 个词，权重为 1–5；通常从 3–4 开始，避免强偏置带来的误识别。只将经过筛选的专业词发送给 ASR 与文本整理模型，不应放入密钥或整段私人聊天。

- ASR：词表放入 `parameters.vocabulary`；领域背景通过音频之前的 `input_text` 消息提供，最多 400 字符。启用 `zh`、`en` 语言提示。
- 整理：同一词表作为拼写参考，提示模型保留陌生术语，不强行翻译、补全路径。疑似错词不是全局替换规则。
- `ASR_EXTRA_FORM_JSON` 仍按顶层字段覆盖请求。原生接口的选项在 `parameters` 中；覆盖整个字段会覆盖默认词表、格式与语言提示，请提供完整参数。
- 修改词表或配置后，需要重启后台服务。`doctor` 显示生效模型、词条数和接口。

可用现有录音进行回归检查，无需录音、复制到剪贴板或覆盖历史记录：

```sh
opentyless-rs transcribe-file /absolute/path/recording.wav --raw-only
opentyless-rs transcribe-file /absolute/path/recording.wav
```

输出 JSON 的 `raw` 和 `polished` 字段。测试应比较同一音频在旧模型、新模型无词表、新模型有词表时的结果。历史 Raw/Polished 不是人工标注的真值；缩写、拼读、路径须听音确认，不能据此宣称准确率。

官方参考：
- https://help.aliyun.com/en/model-studio/qwen-audio-asr-http-api
- https://help.aliyun.com/en/model-studio/improve-asr-accuracy
