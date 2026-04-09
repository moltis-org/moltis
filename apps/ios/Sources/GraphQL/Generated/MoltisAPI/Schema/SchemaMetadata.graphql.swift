// @generated
// This file was automatically generated and should not be edited.

import ApolloAPI

nonisolated protocol MoltisAPI_SelectionSet: ApolloAPI.SelectionSet & ApolloAPI.RootSelectionSet
where Schema == MoltisAPI.SchemaMetadata {}

nonisolated protocol MoltisAPI_InlineFragment: ApolloAPI.SelectionSet & ApolloAPI.InlineFragment
where Schema == MoltisAPI.SchemaMetadata {}

nonisolated protocol MoltisAPI_MutableSelectionSet: ApolloAPI.MutableRootSelectionSet
where Schema == MoltisAPI.SchemaMetadata {}

nonisolated protocol MoltisAPI_MutableInlineFragment: ApolloAPI.MutableSelectionSet & ApolloAPI.InlineFragment
where Schema == MoltisAPI.SchemaMetadata {}

extension MoltisAPI {
  typealias SelectionSet = MoltisAPI_SelectionSet

  typealias InlineFragment = MoltisAPI_InlineFragment

  typealias MutableSelectionSet = MoltisAPI_MutableSelectionSet

  typealias MutableInlineFragment = MoltisAPI_MutableInlineFragment

  nonisolated enum SchemaMetadata: ApolloAPI.SchemaMetadata {
    static let configuration: any ApolloAPI.SchemaConfiguration.Type = SchemaConfiguration.self

    private static let objectTypeMap: [String: ApolloAPI.Object] = [
      "AgentMutation": MoltisAPI.Objects.AgentMutation,
      "BoolResult": MoltisAPI.Objects.BoolResult,
      "ModelInfo": MoltisAPI.Objects.ModelInfo,
      "ModelQuery": MoltisAPI.Objects.ModelQuery,
      "MutationRoot": MoltisAPI.Objects.MutationRoot,
      "QueryRoot": MoltisAPI.Objects.QueryRoot,
      "SessionEntry": MoltisAPI.Objects.SessionEntry,
      "SessionQuery": MoltisAPI.Objects.SessionQuery,
      "StatusInfo": MoltisAPI.Objects.StatusInfo
    ]

    static func objectType(forTypename typename: String) -> ApolloAPI.Object? {
      objectTypeMap[typename]
    }
  }

  nonisolated enum Objects {}
  nonisolated enum Interfaces {}
  nonisolated enum Unions {}

}