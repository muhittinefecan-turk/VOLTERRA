//
// ThroughputChart.swift
// VolterraMac
//

import SwiftUI
import Charts

public struct ThroughputChart: View {
    public let dataPoints: [ThroughputPoint]

    public init(dataPoints: [ThroughputPoint]) {
        self.dataPoints = dataPoints
    }

    public var body: some View {
        if dataPoints.isEmpty {
            VStack {
                Spacer()
                Text("Awaiting transfer activity...")
                    .font(.caption)
                    .foregroundColor(.secondary)
                Spacer()
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            Chart {
                ForEach(dataPoints) { point in
                    AreaMark(
                        x: .value("Time", point.timestamp),
                        y: .value("Instant (MB/s)", point.instantaneousMbps)
                    )
                    .foregroundStyle(
                        LinearGradient(
                            colors: [Color.blue.opacity(0.3), Color.blue.opacity(0.02)],
                            startPoint: .top,
                            endPoint: .bottom
                        )
                    )

                    LineMark(
                        x: .value("Time", point.timestamp),
                        y: .value("Instant (MB/s)", point.instantaneousMbps)
                    )
                    .foregroundStyle(Color.blue)
                    .lineStyle(StrokeStyle(lineWidth: 2))

                    LineMark(
                        x: .value("Time", point.timestamp),
                        y: .value("5s EWMA", point.ewma5sMbps)
                    )
                    .foregroundStyle(Color.green)
                    .lineStyle(StrokeStyle(lineWidth: 1.5, dash: [4, 2]))
                }
            }
            .chartYAxis {
                AxisMarks(position: .leading)
            }
            .chartXAxis(.hidden)
        }
    }
}

public struct MultiLineThroughputChart: View {
    public let dataPoints: [ThroughputPoint]

    public init(dataPoints: [ThroughputPoint]) {
        self.dataPoints = dataPoints
    }

    public var body: some View {
        if dataPoints.isEmpty {
            VStack {
                Spacer()
                Text("Awaiting performance telemetry...")
                    .font(.caption)
                    .foregroundColor(.secondary)
                Spacer()
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            Chart {
                ForEach(dataPoints) { point in
                    LineMark(
                        x: .value("Time", point.timestamp),
                        y: .value("Throughput", point.instantaneousMbps),
                        series: .value("Rate", "1s Instantaneous")
                    )
                    .foregroundStyle(Color.cyan)
                    .lineStyle(StrokeStyle(lineWidth: 1.5))

                    LineMark(
                        x: .value("Time", point.timestamp),
                        y: .value("Throughput", point.ewma5sMbps),
                        series: .value("Rate", "5s EWMA")
                    )
                    .foregroundStyle(Color.blue)
                    .lineStyle(StrokeStyle(lineWidth: 2))

                    LineMark(
                        x: .value("Time", point.timestamp),
                        y: .value("Throughput", point.ewma30sMbps),
                        series: .value("Rate", "30s Smoothed")
                    )
                    .foregroundStyle(Color.indigo)
                    .lineStyle(StrokeStyle(lineWidth: 2))

                    LineMark(
                        x: .value("Time", point.timestamp),
                        y: .value("Throughput", point.physicalThroughputMbps),
                        series: .value("Rate", "Physical Verified")
                    )
                    .foregroundStyle(Color.green)
                    .lineStyle(StrokeStyle(lineWidth: 2, dash: [5, 3]))
                }
            }
            .chartYAxis {
                AxisMarks(position: .leading)
            }
            .chartXAxis(.hidden)
        }
    }
}
