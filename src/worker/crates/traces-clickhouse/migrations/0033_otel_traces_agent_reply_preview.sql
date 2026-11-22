ALTER TABLE {database}.otel_traces
    MODIFY COLUMN ReplyPreview String
        DEFAULT multiIf(
            SpanName = 'claude_code.assistant_response',
            substringUTF8(JSONExtractString(Output, 'content'), 1, 240),
            ParentSpanId = '' AND ObservationType = 'agent',
            substringUTF8(
                if(startsWith(Output, '['),
                   JSONExtractString(arrayElement(JSONExtractArrayRaw(Output), -1), 'content'),
                   JSONExtractString(Output, 'content')), 1, 240),
            '')
