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

## 缩写上下文与可复现检查

`examples/asr/vocabulary.json` 提供通用的中英文计算机词表示例。可将其复制到个人配置目录，按实际使用场景增减。个人完整词表、会话、录音、原始转写和凭据不随仓库发布。

`examples/asr/acronym-context.txt` 提供缩写释义示例，将其单行内容设置为 `ASR_CONTEXT`，然后重启服务。内容解释 TUI、CLI、PR、CI、CD 的英文全称与含义，不要求将相似字符串强制改成这些缩写。

复测时，固定同一录音、模型、词表与其他参数，分别使用空上下文和示例上下文，每组至少重复三次；另留一批录音检查是否退化，并加入不含缩写的录音检查误插入。命令只发送指定录音和配置中的词表/上下文，不读取会话历史：

```sh
ASR_CONTEXT= opentyless-rs transcribe-file /absolute/path/recording.wav --raw-only
ASR_CONTEXT="$(cat examples/asr/acronym-context.txt)" opentyless-rs transcribe-file /absolute/path/recording.wav --raw-only
```

一次本地实验使用 18 条录音，进行了 279 次 ASR 参数对照和 6 次完整识别/整理链路检查。某条 TUI 难例在原配置下 4/8 次出现目标拼写，在术语表加缩写释义上下文下为 8/8。另 10 条留出录音的目标缩写拼写保持原有表现；其中 3 条无目标缩写的录音各重复三次，未观察到误插入。这是小样本重复结果，不是总体准确率；历史转写也不是人工标注真值。

限制：连读的 PR/CI 组合及其邻近词仍可能出错；上下文可能将 `config.toml` 写成口述形式“config 点 TOML”。新增组合热词或仅提高权重未稳定解决这些问题，因此没有加入强制纠错规则。
