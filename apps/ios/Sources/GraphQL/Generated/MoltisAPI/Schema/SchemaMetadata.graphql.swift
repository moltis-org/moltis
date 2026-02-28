// @generated
// This file was automatically generated and should not be edited.

import ApolloAPI

protocol MoltisAPI_SelectionSet: ApolloAPI.SelectionSet & ApolloAPI.RootSelectionSet
where Schema == MoltisAPI.SchemaMetadata {}

protocol MoltisAPI_InlineFragment: ApolloAPI.SelectionSet & ApolloAPI.InlineFragment
where Schema == MoltisAPI.SchemaMetadata {}

protocol MoltisAPI_MutableSelectionSet: ApolloAPI.MutableRootSelectionSet
where Schema == MoltisAPI.SchemaMetadata {}

protocol MoltisAPI_MutableInlineFragment: ApolloAPI.MutableSelectionSet & ApolloAPI.InlineFragment
where Schema == MoltisAPI.SchemaMetadata {}

extension MoltisAPI {
  typealias SelectionSet = MoltisAPI_SelectionSet

  typealias InlineFragment = MoltisAPI_InlineFragment

  typealias MutableSelectionSet = MoltisAPI_MutableSelectionSet

  typealias MutableInlineFragment = MoltisAPI_MutableInlineFragment

  enum SchemaMetadata: ApolloAPI.SchemaMetadata {
    static let configuration: any ApolloAPI.SchemaConfiguration.Type = SchemaConfiguration.self

    static func objectType(forTypename typename: String) -> ApolloAPI.Object? {
      switch typename {
      case "AgentMutation": return MoltisAPI.Objects.AgentMutation
      case "BoolResult": return MoltisAPI.Objects.BoolResult
      case "ModelInfo": return MoltisAPI.Objects.ModelInfo
      case "ModelQuery": return MoltisAPI.Objects.ModelQuery
      case "MutationRoot": return MoltisAPI.Objects.MutationRoot
      case "QueryRoot": return MoltisAPI.Objects.QueryRoot
      case "SessionEntry": return MoltisAPI.Objects.SessionEntry
      case "SessionQuery": return MoltisAPI.Objects.SessionQuery
      case "StatusInfo": return MoltisAPI.Objects.StatusInfo
      default: return nil
      }
    }
  }

  enum Objects {}
  enum Interfaces {}
  enum Unions {}

}