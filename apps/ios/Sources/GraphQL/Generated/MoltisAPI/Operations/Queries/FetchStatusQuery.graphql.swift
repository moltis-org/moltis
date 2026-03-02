// @generated
// This file was automatically generated and should not be edited.

@_exported import ApolloAPI
@_spi(Execution) @_spi(Unsafe) import ApolloAPI

extension MoltisAPI {
  struct FetchStatusQuery: GraphQLQuery {
    static let operationName: String = "FetchStatus"
    static let operationDocument: ApolloAPI.OperationDocument = .init(
      definition: .init(
        #"query FetchStatus { status { __typename hostname version connections uptimeMs } }"#
      ))

    public init() {}

    struct Data: MoltisAPI.SelectionSet {
      let __data: DataDict
      init(_dataDict: DataDict) { __data = _dataDict }

      static var __parentType: any ApolloAPI.ParentType { MoltisAPI.Objects.QueryRoot }
      static var __selections: [ApolloAPI.Selection] { [
        .field("status", Status.self),
      ] }
      static var __fulfilledFragments: [any ApolloAPI.SelectionSet.Type] { [
        FetchStatusQuery.Data.self
      ] }

      /// Gateway status with hostname, version, connections, uptime.
      var status: Status { __data["status"] }

      /// Status
      ///
      /// Parent Type: `StatusInfo`
      struct Status: MoltisAPI.SelectionSet {
        let __data: DataDict
        init(_dataDict: DataDict) { __data = _dataDict }

        static var __parentType: any ApolloAPI.ParentType { MoltisAPI.Objects.StatusInfo }
        static var __selections: [ApolloAPI.Selection] { [
          .field("__typename", String.self),
          .field("hostname", String?.self),
          .field("version", String?.self),
          .field("connections", Int?.self),
          .field("uptimeMs", Int?.self),
        ] }
        static var __fulfilledFragments: [any ApolloAPI.SelectionSet.Type] { [
          FetchStatusQuery.Data.Status.self
        ] }

        var hostname: String? { __data["hostname"] }
        var version: String? { __data["version"] }
        var connections: Int? { __data["connections"] }
        var uptimeMs: Int? { __data["uptimeMs"] }
      }
    }
  }

}