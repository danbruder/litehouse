module Page.Settings exposing
    ( Model
    , Msg
    , init
    , update
    , view
    , save
    , load
    )

import Effect exposing (Effect)
import Html exposing (Html, button, div, h2, h3, input, label, p, span, text)
import Html.Attributes exposing (class, disabled, for, id, placeholder, required, type_, value)
import Html.Events exposing (onClick, onInput)
import Shared


-- MODEL


type alias Model =
    { s3Config : Maybe Effect.S3ConfigRedacted
    , s3ConfigLoading : Bool
    , s3Form : Effect.S3ConfigForm
    , s3FormError : Maybe String
    , s3FormSaving : Bool
    }


-- INIT


init : Shared.Model navigationKey -> ( Model, Effect Msg )
init shared =
    let
        effects =
            case shared.token of
                Just token ->
                    Effect.FetchS3Config token GotS3Config

                Nothing ->
                    Effect.none
    in
    ( { s3Config = Nothing
      , s3ConfigLoading = True
      , s3Form =
            { accessKeyId = ""
            , secretAccessKey = ""
            , bucket = ""
            , region = "us-east-1"
            , endpoint = ""
            , pathPrefix = "litehouse"
            }
      , s3FormError = Nothing
      , s3FormSaving = False
      }
    , effects
    )


-- UPDATE


type Msg
    = GotS3Config (Result String (Maybe Effect.S3ConfigRedacted))
    | S3FormFieldChanged String String
    | SubmitS3Config
    | GotS3ConfigSet (Result String String)
    | DeleteS3Config
    | GotS3ConfigDeleted (Result String String)


update : Shared.Model navigationKey -> Msg -> Model -> ( Model, Effect Msg )
update shared msg model =
    case msg of
        GotS3Config result ->
            case result of
                Ok maybeConfig ->
                    let
                        updatedForm =
                            case maybeConfig of
                                Just config ->
                                    { accessKeyId = config.accessKeyId
                                    , secretAccessKey = ""
                                    , bucket = config.bucket
                                    , region = config.region
                                    , endpoint = Maybe.withDefault "" config.endpoint
                                    , pathPrefix = Maybe.withDefault "litehouse" config.pathPrefix
                                    }

                                Nothing ->
                                    model.s3Form
                    in
                    ( { model
                        | s3Config = maybeConfig
                        , s3ConfigLoading = False
                        , s3Form = updatedForm
                      }
                    , Effect.none
                    )

                Err err ->
                    ( { model
                        | s3ConfigLoading = False
                        , s3FormError = Just err
                      }
                    , Effect.none
                    )

        S3FormFieldChanged fieldName newValue ->
            let
                currentForm =
                    model.s3Form

                updatedForm =
                    case fieldName of
                        "accessKeyId" ->
                            { currentForm | accessKeyId = newValue }

                        "secretAccessKey" ->
                            { currentForm | secretAccessKey = newValue }

                        "bucket" ->
                            { currentForm | bucket = newValue }

                        "region" ->
                            { currentForm | region = newValue }

                        "endpoint" ->
                            { currentForm | endpoint = newValue }

                        "pathPrefix" ->
                            { currentForm | pathPrefix = newValue }

                        _ ->
                            currentForm
            in
            ( { model
                | s3Form = updatedForm
                , s3FormError = Nothing
              }
            , Effect.none
            )

        SubmitS3Config ->
            case shared.token of
                Just token ->
                    let
                        form =
                            model.s3Form
                    in
                    if String.isEmpty (String.trim form.accessKeyId) || String.isEmpty (String.trim form.secretAccessKey) || String.isEmpty (String.trim form.bucket) || String.isEmpty (String.trim form.region) then
                        ( { model
                            | s3FormError = Just "Access Key ID, Secret Access Key, Bucket, and Region are required"
                          }
                        , Effect.none
                        )

                    else
                        ( { model
                            | s3FormSaving = True
                            , s3FormError = Nothing
                          }
                        , Effect.SetS3Config token form GotS3ConfigSet
                        )

                Nothing ->
                    ( model, Effect.none )

        GotS3ConfigSet result ->
            case result of
                Ok _ ->
                    case shared.token of
                        Just token ->
                            ( { model
                                | s3FormSaving = False
                                , s3FormError = Nothing
                              }
                            , Effect.FetchS3Config token GotS3Config
                            )

                        Nothing ->
                            ( { model
                                | s3FormSaving = False
                              }
                            , Effect.none
                            )

                Err err ->
                    ( { model
                        | s3FormSaving = False
                        , s3FormError = Just err
                      }
                    , Effect.none
                    )

        DeleteS3Config ->
            case shared.token of
                Just token ->
                    ( { model
                        | s3FormSaving = True
                        , s3FormError = Nothing
                      }
                    , Effect.DeleteS3Config token GotS3ConfigDeleted
                    )

                Nothing ->
                    ( model, Effect.none )

        GotS3ConfigDeleted result ->
            case result of
                Ok _ ->
                    let
                        emptyForm =
                            { accessKeyId = ""
                            , secretAccessKey = ""
                            , bucket = ""
                            , region = "us-east-1"
                            , endpoint = ""
                            , pathPrefix = "litehouse"
                            }
                    in
                    ( { model
                        | s3Config = Nothing
                        , s3Form = emptyForm
                        , s3FormSaving = False
                        , s3FormError = Nothing
                      }
                    , Effect.none
                    )

                Err err ->
                    ( { model
                        | s3FormSaving = False
                        , s3FormError = Just err
                      }
                    , Effect.none
                    )


-- VIEW


view : Shared.Model navigationKey -> Model -> Html Msg
view shared model =
    div [ class "bg-litehouse-surface rounded-2xl shadow-soft border border-litehouse-border p-6" ]
        [ div [ class "flex justify-between items-center mb-6 pb-4 border-b border-litehouse-border" ]
            [ h2 [ class "text-xl font-semibold text-litehouse-text" ] [ text "Settings" ]
            ]
        , div []
            [ h3 [ class "text-lg font-medium text-litehouse-text mb-4" ] [ text "S3 Backup Configuration" ]
            , p [ class "text-sm text-litehouse-muted mb-6" ]
                [ text "Configure S3-compatible storage for SQLite backups using Litestream." ]
            , if model.s3ConfigLoading then
                viewLoading

              else
                viewS3ConfigForm model
            ]
        ]


viewLoading : Html Msg
viewLoading =
    div [ class "flex items-center justify-center py-12" ]
        [ div [ class "w-10 h-10 border-3 border-litehouse-border border-t-litehouse-amber rounded-full animate-spin-slow" ] []
        ]


viewS3ConfigForm : Model -> Html Msg
viewS3ConfigForm model =
    let
        form =
            model.s3Form

        hasExistingConfig =
            model.s3Config /= Nothing
    in
    div [ class "space-y-6" ]
        [ div [ class "grid grid-cols-2 gap-4" ]
            [ viewFormField "Access Key ID" "accessKeyId" form.accessKeyId "text" False
            , viewFormField "Secret Access Key" "secretAccessKey" form.secretAccessKey "password" False
            ]
        , div [ class "grid grid-cols-2 gap-4" ]
            [ viewFormField "Bucket" "bucket" form.bucket "text" False
            , viewFormField "Region" "region" form.region "text" False
            ]
        , div [ class "grid grid-cols-2 gap-4" ]
            [ viewFormField "Endpoint (optional)" "endpoint" form.endpoint "text" False
            , viewFormField "Path Prefix (optional)" "pathPrefix" form.pathPrefix "text" False
            ]
        , case model.s3FormError of
            Just err ->
                div [ class "px-4 py-3 rounded-xl bg-litehouse-error/10 text-litehouse-error text-sm" ]
                    [ text err ]

            Nothing ->
                text ""
        , div [ class "flex gap-3" ]
            [ button
                [ class "px-4 py-2 rounded-xl bg-litehouse-amber text-litehouse-bg font-medium hover:bg-litehouse-amber/90 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                , onClick SubmitS3Config
                , disabled model.s3FormSaving
                ]
                [ text
                    (if model.s3FormSaving then
                        "Saving..."

                     else if hasExistingConfig then
                        "Update Configuration"

                     else
                        "Save Configuration"
                    )
                ]
            , if hasExistingConfig then
                button
                    [ class "px-4 py-2 rounded-xl border border-litehouse-border text-litehouse-error hover:bg-litehouse-error/10 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                    , onClick DeleteS3Config
                    , disabled model.s3FormSaving
                    ]
                    [ text "Delete Configuration" ]

              else
                text ""
            ]
        ]


viewFormField : String -> String -> String -> String -> Bool -> Html Msg
viewFormField labelText fieldName fieldValue inputType isRequired =
    div []
        [ label [ for fieldName, class "block text-sm font-medium text-litehouse-text mb-2" ]
            [ text labelText ]
        , input
            [ type_ inputType
            , id fieldName
            , value fieldValue
            , onInput (S3FormFieldChanged fieldName)
            , required isRequired
            , placeholder labelText
            , class "w-full px-4 py-2 rounded-xl bg-litehouse-bg border border-litehouse-border text-litehouse-text placeholder-litehouse-muted focus:outline-none focus:ring-2 focus:ring-litehouse-amber/50 transition-shadow"
            ]
            []
        ]


-- PERSISTENCE


save : Model -> Cmd msg
save model =
    Cmd.none


load : (Model -> msg) -> Cmd msg
load toMsg =
    Cmd.none
