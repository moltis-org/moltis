set(LLAMA_VERSION      0.0.0)
set(LLAMA_BUILD_COMMIT unknown)
set(LLAMA_BUILD_NUMBER 0)
set(LLAMA_SHARED_LIB   OFF)


####### Expanded from @PACKAGE_INIT@ by configure_package_config_file() #######
####### Any changes to this file will be overwritten by the next CMake run ####
####### The input file was llama-config.cmake.in                            ########

get_filename_component(PACKAGE_PREFIX_DIR "${CMAKE_CURRENT_LIST_DIR}/../../../" ABSOLUTE)

macro(set_and_check _var _file)
  set(${_var} "${_file}")
  if(NOT EXISTS "${_file}")
    message(FATAL_ERROR "File or directory ${_file} referenced by variable ${_var} does not exist !")
  endif()
endmacro()

macro(check_required_components _NAME)
  foreach(comp ${${_NAME}_FIND_COMPONENTS})
    if(NOT ${_NAME}_${comp}_FOUND)
      if(${_NAME}_FIND_REQUIRED_${comp})
        set(${_NAME}_FOUND FALSE)
      endif()
    endif()
  endforeach()
endmacro()

####################################################################################

set_and_check(LLAMA_INCLUDE_DIR "${PACKAGE_PREFIX_DIR}/include")
set_and_check(LLAMA_LIB_DIR     "${PACKAGE_PREFIX_DIR}/lib64")
set_and_check(LLAMA_BIN_DIR     "${PACKAGE_PREFIX_DIR}/bin")

find_package(ggml REQUIRED HINTS ${LLAMA_LIB_DIR}/cmake)

find_library(llama_LIBRARY llama
    REQUIRED
    HINTS ${LLAMA_LIB_DIR}
    NO_CMAKE_FIND_ROOT_PATH
)

add_library(llama UNKNOWN IMPORTED)
set_target_properties(llama
    PROPERTIES
        INTERFACE_INCLUDE_DIRECTORIES "${LLAMA_INCLUDE_DIR}"
        INTERFACE_LINK_LIBRARIES "ggml::ggml;ggml::ggml-base;"
        IMPORTED_LINK_INTERFACE_LANGUAGES "CXX"
        IMPORTED_LOCATION "${llama_LIBRARY}"
        INTERFACE_COMPILE_FEATURES c_std_90
        POSITION_INDEPENDENT_CODE ON)

check_required_components(Llama)
