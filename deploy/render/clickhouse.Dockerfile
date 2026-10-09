FROM clickhouse/clickhouse-server:26.9.6.6@sha256:eb4870e7ca7ed70c259eebfcfbee6cf797017f6b5436c2926bbbfe3d4d28486e
COPY deploy/clickhouse/keeper.xml /etc/clickhouse-server/config.d/lens-keeper.xml
