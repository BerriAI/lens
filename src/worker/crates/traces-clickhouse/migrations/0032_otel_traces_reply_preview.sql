ALTER TABLE {database}.otel_traces
    ADD COLUMN IF NOT EXISTS ReplyPreview String
        DEFAULT if(SpanName = 'claude_code.assistant_response',
                   substringUTF8(JSONExtractString(Output, 'content'), 1, 240), '')
        AFTER InputPreview
